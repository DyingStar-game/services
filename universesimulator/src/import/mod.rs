//! Idempotent bootstrap of a federation's world from persistence (ADR 0005, 0013).
//!
//! Every `poi_village` under the federation's planet that has at least one `cargo_depot`
//! attached (field `poi_uuid` of the depot's object_data) becomes a village: a commune in
//! social under the federation, and a node of the route graph. Its warehouses are its
//! cargo_depots. Policies, flows, capacities and mineral sources come from profiles (defaults,
//! per village type, per village); routes link each village to its nearest neighbours.
//! Re-running the import never overwrites what an admin changed, except explicit per-village
//! overrides of the world file.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::clients::{Holder, PoiItem, PoliticalEntity};
use crate::db::{catalog::{self, Federation, FederationInput, Resource, ResourceInput}, history, world::{self, CachedEntity, FlowInput, MineralSourceInput, PoiCache, VillageInput, VillagePolicy}};
use crate::error::{AppError, Result};
use crate::state::AppState;

/// Persistence object types.
pub const VILLAGE_TYPE: &str = "poi_village";
pub const DEPOT_TYPE: &str = "cargo_depot";
/// Field of a cargo_depot's object_data naming its village.
pub const DEPOT_VILLAGE_FIELD: &str = "poi_uuid";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldFile {
    pub federation: FederationFile,
    #[serde(default)]
    pub resources: Vec<ResourceInput>,
    #[serde(default)]
    pub villages: VillagesFile,
    #[serde(default)]
    pub routes: RoutesFile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederationFile {
    /// Federation settings (`politicalEntityId` is filled by the import; `planetPoiUuid` is
    /// required).
    #[serde(flatten)]
    pub settings: FederationInput,
    /// Minted once when the treasury is empty.
    #[serde(default)]
    pub initial_treasury: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VillagesFile {
    /// Applied to every village (only what is missing).
    #[serde(default)]
    pub defaults: Profile,
    /// Per persistence `type` of the poi_village (e.g. `factory`), over the defaults.
    #[serde(default)]
    pub by_type: HashMap<String, Profile>,
    /// When the village's `type` has no profile: first letters of its name → a `byType` key
    /// (e.g. `"M": "mining"`). The longest matching prefix wins.
    #[serde(default)]
    pub type_by_name_prefix: HashMap<String, String>,
    /// Per village name (e.g. `M0002`): always applied, over the rest.
    #[serde(default)]
    pub overrides: HashMap<String, Profile>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub policies: Vec<PolicyFile>,
    #[serde(default)]
    pub flows: Vec<FlowFile>,
    #[serde(default)]
    pub capacities: Vec<CapacityFile>,
    #[serde(default)]
    pub mineral_sources: Vec<MineralSourceFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyFile {
    pub resource: String,
    pub min_threshold: i64,
    pub target_level: i64,
    /// Values are per 1000 inhabitants.
    #[serde(default)]
    pub per_thousand: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowFile {
    pub resource: String,
    pub rate_per_hour: f64,
    /// The rate is per 1000 inhabitants.
    #[serde(default)]
    pub per_thousand: bool,
    #[serde(default = "manual")]
    pub source: String,
    pub label: Option<String>,
}

fn manual() -> String {
    "manual".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapacityFile {
    pub resource: String,
    /// Per warehouse; `null`: unlimited.
    pub capacity: Option<i64>,
    /// Credited once to the village's main warehouse.
    #[serde(default)]
    pub initial_stock: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MineralSourceFile {
    pub resource: String,
    pub remaining_units: Option<i64>,
    pub max_units_per_mission: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutesFile {
    /// Each village is linked to this many nearest villages.
    #[serde(default = "three")]
    pub nearest_neighbours: usize,
    /// Neighbours farther than this are not linked, except the nearest one (no village is left
    /// isolated).
    #[serde(default = "far")]
    pub max_distance_m: f64,
    /// Additional routes, by village name.
    #[serde(default)]
    pub extra: Vec<RouteFile>,
}

impl Default for RoutesFile {
    fn default() -> Self {
        Self { nearest_neighbours: three(), max_distance_m: far(), extra: Vec::new() }
    }
}

fn three() -> usize {
    3
}

fn far() -> f64 {
    100_000.0
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteFile {
    pub from: String,
    pub to: String,
    /// Defaults to the straight distance between the villages.
    pub distance_m: Option<f64>,
    pub cost: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub federation_id: Uuid,
    pub political_entity_id: Uuid,
    pub entities_created: usize,
    pub entities_found: usize,
    pub resources: usize,
    pub villages: usize,
    pub warehouses: usize,
    /// Villages of the planet without any attached cargo_depot (not simulated).
    pub villages_without_warehouse: usize,
    /// cargo_depots without a valid `poi_uuid` (not simulated).
    pub unattached_warehouses: Vec<Uuid>,
    pub policies_created: usize,
    pub flows_created: usize,
    pub capacities: usize,
    pub mineral_sources: usize,
    pub routes_created: usize,
    /// Automatic routes no longer wanted (admin routes are never removed).
    pub routes_removed: usize,
    pub initial_stocks_applied: usize,
    pub minted: i64,
}

async fn find_or_create(st: &AppState, name: &str, entity_type: &str, description: Option<&str>, report: &mut ImportReport) -> Result<PoliticalEntity> {
    if let Some(e) = st.clients.social.find_entity_by_name(name).await? {
        if e.entity_type != entity_type {
            return Err(AppError::conflict("ENTITY_TYPE_MISMATCH", format!("{name} exists as {}", e.entity_type)));
        }
        report.entities_found += 1;
        return Ok(e);
    }
    report.entities_created += 1;
    st.clients.social.create_entity(entity_type, name, description).await
}

fn position(item: &PoiItem) -> Option<[f64; 3]> {
    item.snapshot().position
}

fn distance(a: Option<[f64; 3]>, b: Option<[f64; 3]>) -> Option<f64> {
    let (a, b) = (a?, b?);
    Some(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt())
}

/// Merges profiles: later ones replace earlier entries of the same resource.
fn merge(profiles: &[&Profile]) -> Profile {
    fn by_resource<T: Clone>(lists: impl Iterator<Item = Vec<T>>, key: impl Fn(&T) -> String) -> Vec<T> {
        let mut map = BTreeMap::new();
        for list in lists {
            for item in list {
                map.insert(key(&item), item);
            }
        }
        map.into_values().collect()
    }
    Profile {
        policies: by_resource(profiles.iter().map(|p| p.policies.clone()), |p| p.resource.clone()),
        flows: by_resource(profiles.iter().map(|p| p.flows.clone()), |f| format!("{}|{}", f.resource, f.source)),
        capacities: by_resource(profiles.iter().map(|p| p.capacities.clone()), |c| c.resource.clone()),
        mineral_sources: by_resource(profiles.iter().map(|p| p.mineral_sources.clone()), |s| s.resource.clone()),
    }
}

/// `byType` key of a village: its persistence type when a profile exists for it, else the
/// longest name prefix mapped in `typeByNamePrefix`.
fn profile_key<'a>(villages: &'a VillagesFile, village_type: Option<&'a str>, name: &str) -> Option<&'a str> {
    if let Some(t) = village_type.filter(|t| villages.by_type.contains_key(*t)) {
        return Some(t);
    }
    villages
        .type_by_name_prefix
        .iter()
        .filter(|(prefix, _)| name.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, key)| key.as_str())
}

fn scaled(value: f64, per_thousand: bool, population: i64) -> f64 {
    if per_thousand { value * population as f64 / 1000.0 } else { value }
}

/// Lists the planet's villages and depots, groups depots by village (`poi_uuid`).
async fn read_planet(st: &AppState, planet: Uuid) -> Result<(Vec<PoiItem>, HashMap<Uuid, Vec<PoiItem>>, Vec<Uuid>)> {
    let villages = st.clients.persistence.list_items(VILLAGE_TYPE, planet).await?;
    let known: std::collections::HashSet<Uuid> = villages.iter().map(|v| v.uuid).collect();
    let mut depots: HashMap<Uuid, Vec<PoiItem>> = HashMap::new();
    let mut unattached = Vec::new();
    for d in st.clients.persistence.list_items(DEPOT_TYPE, planet).await? {
        match d.uuid_field(DEPOT_VILLAGE_FIELD).filter(|v| known.contains(v)) {
            Some(v) => depots.entry(v).or_default().push(d),
            None => unattached.push(d.uuid),
        }
    }
    for list in depots.values_mut() {
        list.sort_by_key(|d| d.uuid);
    }
    Ok((villages, depots, unattached))
}

fn depot_code(village: &str, index: usize) -> String {
    format!("{village}-depot-{}", index + 1)
}

/// Attaches the depots of an existing village; the first one becomes the main warehouse.
async fn save_depots(st: &AppState, village: Uuid, village_name: &str, depots: &[PoiItem]) -> Result<Vec<Uuid>> {
    let mut ids = Vec::new();
    for (i, d) in depots.iter().enumerate() {
        let snap = d.snapshot();
        let poi = PoiCache { name: format!("{village_name} — {}", snap.name), scenename: snap.scenename, position: snap.position };
        let w = world::upsert_warehouse(&st.pool, d.uuid, village, &depot_code(village_name, i), &poi).await?;
        ids.push(w.id);
    }
    if let Some(first) = ids.first() {
        world::set_main_warehouse(&st.pool, village, *first, false).await?;
    }
    Ok(ids)
}

/// Periodic refresh (tick): attaches depots newly linked to an already simulated village.
pub async fn sync_warehouses(st: &AppState, fed: &Federation) -> Result<usize> {
    let Some(planet) = fed.planet_poi_uuid else { return Ok(0) };
    let (_, depots, _) = read_planet(st, planet).await?;
    let mut count = 0;
    for v in world::active_villages(&st.pool, fed.id).await? {
        if let Some(list) = depots.get(&v.id) {
            count += save_depots(st, v.id, &v.name, list).await?.len();
        }
    }
    Ok(count)
}

/// Imports a world file and keeps it for the periodic replay ([`replay`]).
pub async fn import_and_keep(st: &AppState, raw: serde_json::Value) -> Result<ImportReport> {
    let file: WorldFile = serde_json::from_value(raw.clone()).map_err(|e| AppError::BadRequest(format!("invalid world file: {e}")))?;
    let report = import(st, file).await?;
    sqlx::query(
        "INSERT INTO world_files (federation_id, content, imported_at) VALUES ($1, $2, now())
         ON CONFLICT (federation_id) DO UPDATE SET content = $2, imported_at = now()",
    )
    .bind(report.federation_id)
    .bind(&raw)
    .execute(&st.pool)
    .await?;
    Ok(report)
}

/// Tick: replays the last imported world file of a federation (new villages and depots).
pub async fn replay(st: &AppState, fed: &Federation) -> Result<Option<ImportReport>> {
    let raw: Option<serde_json::Value> = sqlx::query_scalar("SELECT content FROM world_files WHERE federation_id = $1")
        .bind(fed.id)
        .fetch_optional(&st.pool)
        .await?;
    let Some(raw) = raw else { return Ok(None) };
    let file: WorldFile = serde_json::from_value(raw).map_err(|e| AppError::BadRequest(format!("stored world file: {e}")))?;
    Ok(Some(import(st, file).await?))
}

pub async fn import(st: &AppState, file: WorldFile) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let mut settings = file.federation.settings;
    let fed_name = settings.name.clone().ok_or_else(|| AppError::BadRequest("federation.name is required".into()))?;
    if settings.code.is_none() || settings.planet_internal_name.is_none() {
        return Err(AppError::BadRequest("federation.code and federation.planetInternalName are required".into()));
    }
    let planet = settings.planet_poi_uuid.ok_or_else(|| AppError::BadRequest("federation.planetPoiUuid is required".into()))?;

    // 1. Social and simulator: the federation.
    let fed_entity = find_or_create(st, &fed_name, "federation", None, &mut report).await?;
    settings.political_entity_id = Some(fed_entity.id);
    let fed = catalog::upsert_federation(&st.pool, None, &settings).await?;
    report.federation_id = fed.id;
    report.political_entity_id = fed_entity.id;
    world::cache_entity(&st.pool, &CachedEntity { id: fed_entity.id, r#type: fed_entity.entity_type.clone(), name: fed_name.clone(), parent_id: fed_entity.parent_id, federation_id: Some(fed.id) }).await?;

    // 2. Economie: treasury, minting rights, initial money.
    st.clients.economie.ensure_political_wallet(fed_entity.id).await?;
    st.clients.economie.set_minting(fed_entity.id, true, fed.mint_ceiling).await?;
    if file.federation.initial_treasury > 0 && st.clients.economie.political_balance(fed_entity.id, &fed.currency).await? == 0 {
        st.clients.economie.mint(fed_entity.id, file.federation.initial_treasury, &fed.currency, "initial treasury").await?;
        history::record_mint(&st.pool, fed.id, file.federation.initial_treasury, "initial treasury").await?;
        report.minted = file.federation.initial_treasury;
    }

    // 3. Resources.
    for r in &file.resources {
        catalog::upsert_resource(&st.pool, r).await?;
        report.resources += 1;
    }
    let resources: HashMap<String, Resource> = catalog::list_resources(&st.pool).await?.into_iter().map(|r| (r.code.clone(), r)).collect();
    let resource = |code: &str| resources.get(code).ok_or_else(|| AppError::BadRequest(format!("unknown resource {code}")));

    // 4. Villages and warehouses from persistence.
    let (villages, depots, unattached) = read_planet(st, planet).await?;
    report.unattached_warehouses = unattached;
    let mut positions: Vec<(Uuid, String, Option<[f64; 3]>)> = Vec::new();
    for item in &villages {
        let Some(list) = depots.get(&item.uuid) else {
            report.villages_without_warehouse += 1;
            continue;
        };
        let snap = item.snapshot();
        let village_type = item.data["type"].as_str().map(str::to_string);
        let population = item.data["population"].as_i64().unwrap_or(0);
        let entity = find_or_create(st, &snap.name, "commune", None, &mut report).await?;
        if entity.parent_id != Some(fed_entity.id) && entity.parent_id.is_none() {
            st.clients.social.set_parent(entity.id, Some(fed_entity.id)).await?;
        }
        world::cache_entity(&st.pool, &CachedEntity { id: entity.id, r#type: "commune".into(), name: snap.name.clone(), parent_id: entity.parent_id.or(Some(fed_entity.id)), federation_id: Some(fed.id) }).await?;
        let input = VillageInput {
            id: item.uuid,
            code: snap.name.clone(),
            name: snap.name.clone(),
            federation_id: fed.id,
            political_entity_id: entity.id,
            village_type: village_type.clone(),
            population,
            radius_m: item.data["radius_m"].as_f64(),
            position: snap.position,
        };
        world::upsert_village(&st.pool, &input).await?;
        let warehouses = save_depots(st, item.uuid, &snap.name, list).await?;
        report.villages += 1;
        report.warehouses += warehouses.len();
        positions.push((item.uuid, snap.name.clone(), position(item)));

        // 5. Profiles: defaults and type only fill what is missing; overrides are forced.
        let empty = Profile::default();
        let overrides = file.villages.overrides.get(&snap.name).unwrap_or(&empty);
        let profile_key = profile_key(&file.villages, village_type.as_deref(), &snap.name);
        let profile = merge(&[&file.villages.defaults, profile_key.and_then(|t| file.villages.by_type.get(t)).unwrap_or(&empty), overrides]);
        let forced = |resource: &str, list: &dyn Fn(&Profile) -> Vec<String>| list(overrides).iter().any(|r| r == resource);
        for p in &profile.policies {
            let policy = VillagePolicy {
                village_id: item.uuid,
                resource_id: resource(&p.resource)?.id,
                min_threshold: scaled(p.min_threshold as f64, p.per_thousand, population).round() as i64,
                target_level: scaled(p.target_level as f64, p.per_thousand, population).round() as i64,
            };
            if forced(&p.resource, &|o| o.policies.iter().map(|x| x.resource.clone()).collect()) {
                world::upsert_policy(&st.pool, &policy).await?;
                report.policies_created += 1;
            } else if world::insert_policy_if_missing(&st.pool, &policy).await? {
                report.policies_created += 1;
            }
        }
        for f in &profile.flows {
            let flow = FlowInput {
                village_id: item.uuid,
                resource_id: resource(&f.resource)?.id,
                rate_per_hour: scaled(f.rate_per_hour, f.per_thousand, population),
                source: f.source.clone(),
                label: f.label.clone(),
            };
            if world::insert_flow_if_missing(&st.pool, &flow).await? {
                report.flows_created += 1;
            }
        }
        for s in &profile.mineral_sources {
            let source = MineralSourceInput {
                village_id: item.uuid,
                resource_id: resource(&s.resource)?.id,
                remaining_units: s.remaining_units,
                max_units_per_mission: s.max_units_per_mission,
            };
            let saved = if forced(&s.resource, &|o| o.mineral_sources.iter().map(|x| x.resource.clone()).collect()) {
                world::upsert_mineral_source(&st.pool, &source).await.map(|_| true)?
            } else {
                world::insert_mineral_source_if_missing(&st.pool, &source).await?
            };
            report.mineral_sources += saved as usize;
        }
        for c in &profile.capacities {
            let res = resource(&c.resource)?;
            let force = forced(&c.resource, &|o| o.capacities.iter().map(|x| x.resource.clone()).collect());
            for (i, w) in warehouses.iter().enumerate() {
                let initial = if i == 0 { c.initial_stock } else { 0 };
                let cap = if force {
                    world::force_capacity(&st.pool, *w, res.id, c.capacity, initial).await?
                } else {
                    world::insert_capacity_if_missing(&st.pool, *w, res.id, c.capacity, initial).await?
                };
                report.capacities += 1;
                if !cap.initial_stock_applied && cap.initial_stock > 0 {
                    st.clients.inventory.credit(Holder::warehouse(*w), &res.inventory_good_type, cap.initial_stock).await?;
                    world::mark_initial_stock_applied(&st.pool, *w, res.id).await?;
                    report.initial_stocks_applied += 1;
                }
            }
        }
    }

    // 6. Routes: nearest neighbours and the extra ones; the automatic routes no longer wanted
    //    (e.g. a long link made obsolete by a new village in between) are removed.
    let pair = |a: Uuid, b: Uuid| if a < b { (a, b) } else { (b, a) };
    let mut wanted: HashMap<(Uuid, Uuid), (f64, Option<f64>)> = HashMap::new();
    for (id, _, pos) in &positions {
        let mut near: Vec<(f64, Uuid)> = positions
            .iter()
            .filter(|(other, _, _)| other != id)
            .filter_map(|(other, _, p)| distance(*pos, *p).map(|d| (d, *other)))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The nearest neighbour is always linked (no isolated village); the others only
        // within `maxDistanceM`.
        let linked = near
            .iter()
            .enumerate()
            .filter(|(i, (d, _))| *i == 0 || *d <= file.routes.max_distance_m)
            .map(|(_, n)| *n)
            .take(file.routes.nearest_neighbours.max(1));
        for (d, other) in linked {
            wanted.entry(pair(*id, other)).or_insert((d.max(1.0), None));
        }
    }
    let by_name: HashMap<&str, (Uuid, Option<[f64; 3]>)> = positions.iter().map(|(id, n, p)| (n.as_str(), (*id, *p))).collect();
    for r in &file.routes.extra {
        let (from, fp) = *by_name.get(r.from.as_str()).ok_or_else(|| AppError::BadRequest(format!("unknown village {}", r.from)))?;
        let (to, tp) = *by_name.get(r.to.as_str()).ok_or_else(|| AppError::BadRequest(format!("unknown village {}", r.to)))?;
        let distance_m = r.distance_m.or_else(|| distance(fp, tp)).ok_or_else(|| AppError::BadRequest(format!("distance of {} → {} unknown", r.from, r.to)))?;
        wanted.insert(pair(from, to), (distance_m.max(1.0), r.cost));
    }
    for ((a, b), (distance_m, cost)) in &wanted {
        if world::ensure_auto_route(&st.pool, *a, *b, *distance_m, *cost).await? {
            report.routes_created += 1;
        }
    }
    report.routes_removed = world::prune_auto_routes(&st.pool, fed.id, &wanted.keys().copied().collect()).await?;
    info!(federation = %fed.code, villages = report.villages, warehouses = report.warehouses, "world imported");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(resource: &str, min: i64) -> PolicyFile {
        PolicyFile { resource: resource.into(), min_threshold: min, target_level: min * 2, per_thousand: false }
    }

    #[test]
    fn later_profiles_win_per_resource() {
        let a = Profile { policies: vec![policy("water", 10), policy("food", 5)], ..Default::default() };
        let b = Profile { policies: vec![policy("water", 99)], ..Default::default() };
        let m = merge(&[&a, &b]);
        let water = m.policies.iter().find(|p| p.resource == "water").unwrap();
        assert_eq!(water.min_threshold, 99);
        assert_eq!(m.policies.len(), 2);
    }

    #[test]
    fn seed_world_file_parses() {
        let file: WorldFile = serde_json::from_str(include_str!("../../seeds/tarsis_3-ares.json")).unwrap();
        assert!(file.federation.settings.planet_poi_uuid.is_some());
        assert!(file.villages.by_type.contains_key("mining"));
    }

    #[test]
    fn profile_by_type_then_name_prefix() {
        let file: VillagesFile = serde_json::from_value(serde_json::json!({
            "byType": { "mining": {}, "factory": {} },
            "typeByNamePrefix": { "M": "mining", "F": "factory" }
        }))
        .unwrap();
        assert_eq!(profile_key(&file, Some("factory"), "M0001"), Some("factory"));
        assert_eq!(profile_key(&file, Some("poi_village"), "M0058"), Some("mining"));
        assert_eq!(profile_key(&file, None, "F0007"), Some("factory"));
        assert_eq!(profile_key(&file, Some("poi_village"), "C0001"), None);
    }

    #[test]
    fn per_thousand_scaling() {
        assert_eq!(scaled(10.0, true, 2500), 25.0);
        assert_eq!(scaled(10.0, false, 2500), 10.0);
    }
}
