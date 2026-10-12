//! Integration tests: real PostgreSQL (`#[sqlx::test]`, needs `DATABASE_URL`), in-memory
//! stand-in for the other services. World: one planet, a mining village (V1), a town (V2) and
//! a city with two cargo depots (V3), linked V1 — V2 — V3; plus an unattached depot and a
//! village without depot.

use std::sync::Arc;

use axum::{body::Body, http::{Request, StatusCode}};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::auth::Auth;
use crate::clients::{fake::FakeWorld, Clients, Holder};
use crate::config::{Config, ExternalMode};
use crate::db::{catalog, history, orders, world};
use crate::import::{self, WorldFile};
use crate::simulation;
use crate::state::AppState;

const PLANET: u128 = 0x100;
const V1: u128 = 0xA1;
const V2: u128 = 0xA2;
const V3: u128 = 0xA3;
const V4: u128 = 0xA4;
const D1: u128 = 0xD1;
const D2: u128 = 0xD2;
const D3A: u128 = 0xD3A;
const D3B: u128 = 0xD3B;
const D9: u128 = 0xD9;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn config() -> Config {
    Config {
        database_url: String::new(),
        bind: String::new(),
        app_env: "test".into(),
        sim_enabled: false,
        sim_tick_seconds: 300,
        // No flows unless a test enables them.
        sim_time_scale: 0.0,
        sim_leg_max_attempts: 2,
        sim_political_refresh_minutes: 15,
        sim_history_days: 14,
        oidc_issuer: "http://keycloak.test/realms/dyingstar".into(),
        oidc_jwks_url: "http://keycloak.test/certs".into(),
        oidc_audience: None,
        oidc_service_audience: "universesimulator-api".into(),
        internal_service_clients: vec!["svc-game".into()],
        internal_api_key: Some("test-key".into()),
        auth_dev_bypass: true,
        internal_dev_bypass: true,
        external_mode: ExternalMode::DryRun,
        oidc_token_url: String::new(),
        service_client_id: "svc-universesimulator".into(),
        service_client_secret: None,
        outbound_internal_key: None,
        social_api_url: String::new(),
        inventory_api_url: String::new(),
        mission_api_url: String::new(),
        economie_api_url: String::new(),
        persistence_api_url: String::new(),
        persistence_validate: true,
        http_timeout_ms: 1000,
    }
}

fn state(pool: PgPool, fake: Arc<FakeWorld>, cfg: Config) -> AppState {
    AppState { pool, clients: Clients::from_fake(fake), auth: Arc::new(Auth::new(&cfg)), cfg: Arc::new(cfg) }
}

fn pos(x: f64) -> Value {
    json!({ "x": x, "y": 0.0, "z": 0.0 })
}

/// Persistence objects of the planet.
fn populate(fake: &FakeWorld) {
    let p = id(PLANET);
    fake.add_item(id(V1), "poi_village", p, json!({ "name": "M0001", "type": "mining", "population": 500, "radius_m": 2000.0, "position": pos(0.0) }));
    fake.add_item(id(V2), "poi_village", p, json!({ "name": "F0002", "type": "factory", "population": 2000, "radius_m": 2000.0, "position": pos(10_000.0) }));
    fake.add_item(id(V3), "poi_village", p, json!({ "name": "C0003", "type": "factory", "population": 10000, "radius_m": 3000.0, "position": pos(30_000.0) }));
    fake.add_item(id(V4), "poi_village", p, json!({ "name": "M0004", "type": "mining", "population": 100, "position": pos(90_000.0) }));
    fake.add_item(id(D1), "cargo_depot", p, json!({ "name": "cargo_depot", "poi_uuid": id(V1).to_string(), "position": pos(10.0) }));
    fake.add_item(id(D2), "cargo_depot", p, json!({ "name": "cargo_depot", "poi_uuid": id(V2).to_string(), "position": pos(10_010.0) }));
    fake.add_item(id(D3A), "cargo_depot", p, json!({ "name": "cargo_depot", "poi_uuid": id(V3).to_string(), "position": pos(30_010.0) }));
    fake.add_item(id(D3B), "cargo_depot", p, json!({ "name": "cargo_depot", "poi_uuid": id(V3).to_string(), "position": pos(30_020.0) }));
    fake.add_item(id(D9), "cargo_depot", p, json!({ "name": "cargo_depot", "position": pos(50_000.0) }));
    fake.add_item(Uuid::from_u128(0xE1), "mining_depot", p, json!({ "name": "MiningDepot", "position": pos(20.0) }));
}

fn world_file(initial_treasury: i64) -> WorldFile {
    serde_json::from_str(&world_json(initial_treasury)).unwrap()
}

fn world_json(initial_treasury: i64) -> String {
    json!({
        "federation": {
            "code": "ARES", "name": "ARES", "planetInternalName": "tarsis_3", "planetPoiUuid": id(PLANET),
            "initialTreasury": initial_treasury, "treasuryFloor": 0, "mintBatch": 100000, "elasticity": 1.0
        },
        "resources": [
            { "code": "water", "name": "Water", "category": "water", "unit": "l", "inventoryGoodType": "water", "massPerUnitKg": 1.0, "basePrice": 2.0 },
            { "code": "ore_fer", "name": "Iron ore", "category": "ore", "unit": "kg", "inventoryGoodType": "ore_fer", "massPerUnitKg": 1.0, "basePrice": 10.0 }
        ],
        "villages": {
            "defaults": {
                "policies": [{ "resource": "water", "minThreshold": 100, "targetLevel": 300 }],
                "capacities": [{ "resource": "water", "capacity": 1000, "initialStock": 50 }]
            },
            "typeByNamePrefix": { "M": "mining" },
            "byType": {
                "mining": { "flows": [{ "resource": "water", "ratePerHour": -20, "perThousand": true, "source": "population" }] }
            },
            "overrides": {
                "C0003": {
                    "policies": [{ "resource": "water", "minThreshold": 100, "targetLevel": 500 }],
                    "capacities": [{ "resource": "water", "capacity": null, "initialStock": 2000 }]
                },
                "M0001": {
                    "mineralSources": [{ "resource": "ore_fer", "remainingUnits": 1000, "maxUnitsPerMission": 200 }]
                }
            }
        },
        "routes": { "nearestNeighbours": 1, "maxDistanceM": 50000 }
    })
    .to_string()
}

async fn setup(pool: PgPool, treasury: i64) -> (AppState, Arc<FakeWorld>) {
    let fake = Arc::new(FakeWorld::new());
    populate(&fake);
    let st = state(pool, fake.clone(), config());
    import::import(&st, world_file(treasury)).await.unwrap();
    (st, fake)
}

fn water(fake: &FakeWorld, n: u128) -> i64 {
    fake.quantity(Holder::warehouse(id(n)), "water")
}

async fn legs(st: &AppState) -> Vec<orders::Leg> {
    let (orders, _) = orders::list_orders(&st.pool, &Default::default()).await.unwrap();
    let ids: Vec<Uuid> = orders.iter().map(|o| o.id).collect();
    orders::legs_of_orders(&st.pool, &ids).await.unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn import_builds_villages_from_persistence(pool: PgPool) {
    let fake = Arc::new(FakeWorld::new());
    populate(&fake);
    let st = state(pool, fake.clone(), config());
    let r = import::import(&st, world_file(1_000_000)).await.unwrap();
    assert_eq!(r.villages, 3);
    assert_eq!(r.warehouses, 4);
    assert_eq!(r.villages_without_warehouse, 1);
    assert_eq!(r.unattached_warehouses, vec![id(D9)]);
    assert_eq!(r.routes_created, 2, "V1—V2 and V3—V2 (V2's nearest already linked)");

    let city = world::get_village(&st.pool, id(V3)).await.unwrap().unwrap();
    assert_eq!(city.code, "C0003");
    assert_eq!(city.population, 10000);
    assert_eq!(city.main_warehouse_id, Some(id(D3A)), "first depot by id");
    // Initial stock goes to the main warehouse only; the city override replaces the default.
    assert_eq!(water(&fake, D3A), 2000);
    assert_eq!(water(&fake, D3B), 0);
    assert_eq!(water(&fake, D1), 50);
    let city_policy = world::get_policy(&st.pool, id(V3), catalog::find_resource(&st.pool, Some("water"), None).await.unwrap().unwrap().id).await.unwrap().unwrap();
    assert_eq!(city_policy.target_level, 500);
    // Mining type flow scaled by population (500 → -10/h).
    let flows = world::list_flows(&st.pool, Some(id(V1)), None).await.unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].rate_per_hour, -10.0);
    // Villages are communes under ARES.
    let commune = fake.with(|s| s.entities.values().find(|e| e.name == "M0001").cloned()).unwrap();
    let ares = fake.with(|s| s.entities.values().find(|e| e.name == "ARES").cloned()).unwrap();
    assert_eq!(commune.entity_type, "commune");
    assert_eq!(commune.parent_id, Some(ares.id));

    // Idempotent; admin changes survive a re-import (defaults only fill what is missing).
    let water_id = city_policy.resource_id;
    world::upsert_policy(&st.pool, &world::VillagePolicy { village_id: id(V2), resource_id: water_id, min_threshold: 7, target_level: 9 }).await.unwrap();
    let again = import::import(&st, world_file(1_000_000)).await.unwrap();
    assert_eq!(again.entities_created, 0);
    assert_eq!(again.initial_stocks_applied, 0);
    assert_eq!(again.routes_created, 0);
    assert_eq!(again.minted, 0);
    assert_eq!(water(&fake, D3A), 2000);
    assert_eq!(world::get_policy(&st.pool, id(V2), water_id).await.unwrap().unwrap().min_threshold, 7);
}

#[sqlx::test(migrations = "./migrations")]
async fn new_depots_are_attached_by_the_tick(pool: PgPool) {
    let (st, fake) = setup(pool, 1_000_000).await;
    fake.add_item(Uuid::from_u128(0xD22), "cargo_depot", id(PLANET), json!({ "name": "cargo_depot", "poi_uuid": id(V2).to_string(), "position": pos(10_030.0) }));
    simulation::tick(&st, true).await.unwrap();
    let ws = world::list_warehouses(&st.pool, &world::WarehouseFilter { village: Some(id(V2)), ..Default::default() }).await.unwrap();
    assert_eq!(ws.len(), 2);
    assert_eq!(world::get_village(&st.pool, id(V2)).await.unwrap().unwrap().main_warehouse_id, Some(id(D2)), "main unchanged");
}

#[sqlx::test(migrations = "./migrations")]
async fn new_villages_appear_at_the_next_forced_tick(pool: PgPool) {
    let fake = Arc::new(FakeWorld::new());
    populate(&fake);
    let st = state(pool, fake.clone(), config());
    let raw = serde_json::to_value(serde_json::from_str::<Value>(&world_json(1_000_000)).unwrap()).unwrap();
    import::import_and_keep(&st, raw).await.unwrap();
    assert!(world::get_village(&st.pool, id(V4)).await.unwrap().is_none(), "no depot yet");
    // The game links a depot to M0004 (persistence type `poi_village`): mining by name prefix.
    fake.add_item(Uuid::from_u128(0xD4), "cargo_depot", id(PLANET), json!({ "name": "cargo_depot", "poi_uuid": id(V4).to_string(), "position": pos(90_010.0) }));
    fake.add_item(id(V4), "poi_village", id(PLANET), json!({ "name": "M0004", "type": "poi_village", "population": 100, "position": pos(90_000.0) }));
    simulation::tick(&st, true).await.unwrap();
    let v4 = world::get_village(&st.pool, id(V4)).await.unwrap().expect("created by the tick");
    // 60 km away from V3, beyond maxDistanceM: still linked to its nearest neighbour.
    assert!(world::route_exists(&st.pool, id(V4), id(V3)).await.unwrap());

    // A village appears in between: the long automatic link is replaced; an admin route stays.
    let admin_route = world::upsert_route(&st.pool, &world::TravelRouteInput { from_village_id: id(V1), to_village_id: id(V3), distance_m: 30_000.0, cost: None, bidirectional: true, active: true }).await.unwrap();
    fake.add_item(Uuid::from_u128(0xA5), "poi_village", id(PLANET), json!({ "name": "F0005", "type": "factory", "population": 1000, "position": pos(60_000.0) }));
    fake.add_item(Uuid::from_u128(0xD5), "cargo_depot", id(PLANET), json!({ "name": "cargo_depot", "poi_uuid": Uuid::from_u128(0xA5).to_string(), "position": pos(60_010.0) }));
    simulation::tick(&st, true).await.unwrap();
    assert!(!world::route_exists(&st.pool, id(V4), id(V3)).await.unwrap(), "obsolete long link removed");
    assert!(world::route_exists(&st.pool, id(V4), Uuid::from_u128(0xA5)).await.unwrap());
    let routes = world::list_routes(&st.pool, None).await.unwrap();
    assert!(routes.iter().any(|r| r.id == admin_route.id && !r.auto), "admin route kept");
    assert_eq!(v4.main_warehouse_id, Some(Uuid::from_u128(0xD4)));
    let flows = world::list_flows(&st.pool, Some(id(V4)), None).await.unwrap();
    assert_eq!(flows.len(), 1, "mining profile through the M prefix");
}

#[sqlx::test(migrations = "./migrations")]
async fn transport_order_goes_village_by_village(pool: PgPool) {
    let (st, fake) = setup(pool, 1_000_000).await;

    // V1 and V2 are below their threshold (50 < 100); the city supplies.
    let r = simulation::tick(&st, true).await.unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let all = legs(&st).await;
    let to_v1: Vec<_> = {
        let (orders, _) = orders::list_orders(&st.pool, &orders::OrderFilter { kind: Some("transport".into()), ..Default::default() }).await.unwrap();
        let o = orders.iter().find(|o| o.destination_village_id == id(V1)).unwrap();
        assert_eq!(o.path, vec![id(V3), id(V2), id(V1)]);
        all.iter().filter(|l| l.order_id == o.id).cloned().collect()
    };
    let (first, second) = (&to_v1[0], &to_v1[1]);
    assert_eq!((first.from_village_id, first.to_village_id), (Some(id(V3)), id(V2)));
    assert_eq!(first.quantity, 250);
    assert_eq!(first.status, "published");
    // Held in the city warehouse that has the goods, delivered to V2's main warehouse.
    assert_eq!(first.from_warehouse_id, Some(id(D3A)));
    assert_eq!(first.to_warehouse_id, Some(id(D2)));
    assert_eq!(second.status, "waiting");
    let mission = fake.with(|s| s.missions[&first.mission_id.unwrap()].mission.clone());
    assert_eq!(mission.objectives[0]["locationTo"]["village"], "F0002");
    assert_eq!(mission.script_id, format!("usim:leg:{}:0", first.id));

    fake.complete_mission(first.mission_id.unwrap());
    simulation::tick(&st, true).await.unwrap();
    let second = legs(&st).await.into_iter().find(|l| l.id == second.id).unwrap();
    assert_eq!(second.status, "published");
    assert_eq!(second.from_warehouse_id, Some(id(D2)), "taken where the first leg delivered");
    assert_eq!(second.to_warehouse_id, Some(id(D1)));

    fake.complete_mission(second.mission_id.unwrap());
    simulation::tick(&st, true).await.unwrap();
    assert_eq!(water(&fake, D1), 300);
    let order = orders::get_order(&st.pool, second.order_id).await.unwrap().unwrap();
    assert_eq!(order.status, "completed");
}

#[sqlx::test(migrations = "./migrations")]
async fn mining_order_delivers_to_the_main_warehouse(pool: PgPool) {
    let (st, fake) = setup(pool, 1_000_000).await;
    let ore = catalog::find_resource(&st.pool, Some("ore_fer"), None).await.unwrap().unwrap();
    world::upsert_policy(&st.pool, &world::VillagePolicy { village_id: id(V1), resource_id: ore.id, min_threshold: 100, target_level: 500 }).await.unwrap();
    simulation::tick(&st, true).await.unwrap();
    let leg = legs(&st).await.into_iter().find(|l| l.from_village_id.is_none()).unwrap();
    assert_eq!(leg.quantity, 200);
    assert_eq!(leg.to_warehouse_id, Some(id(D1)));
    let mission = fake.with(|s| s.missions[&leg.mission_id.unwrap()].mission.clone());
    assert_eq!(mission.objectives[0]["params"]["to"]["holderId"], id(D1).to_string());

    fake.complete_mission(leg.mission_id.unwrap());
    simulation::tick(&st, true).await.unwrap();
    assert_eq!(fake.quantity(Holder::warehouse(id(D1)), "ore_fer"), 200);
    let source = world::list_mineral_sources(&st.pool, None, Some(id(V1))).await.unwrap().remove(0);
    assert_eq!(source.remaining_units, Some(800));
}

#[sqlx::test(migrations = "./migrations")]
async fn remote_shortage_is_mined_where_the_ore_is(pool: PgPool) {
    let (st, fake) = setup(pool, 1_000_000).await;
    let ore = catalog::find_resource(&st.pool, Some("ore_fer"), None).await.unwrap().unwrap();
    // The city needs ore; only the mining village (V1, no ore policy) has a source.
    world::upsert_policy(&st.pool, &world::VillagePolicy { village_id: id(V3), resource_id: ore.id, min_threshold: 100, target_level: 300 }).await.unwrap();
    simulation::tick(&st, true).await.unwrap();
    let (mining, _) = orders::list_orders(&st.pool, &orders::OrderFilter { kind: Some("mining".into()), ..Default::default() }).await.unwrap();
    assert_eq!(mining.len(), 1);
    assert_eq!(mining[0].destination_village_id, id(V1));
    assert_eq!(mining[0].quantity, 200, "capped by maxUnitsPerMission");
    // Mined: the ore is V1's surplus and is shipped to the city.
    let leg = legs(&st).await.into_iter().find(|l| l.order_id == mining[0].id).unwrap();
    fake.complete_mission(leg.mission_id.unwrap());
    simulation::tick(&st, true).await.unwrap();
    let (transport, _) = orders::list_orders(&st.pool, &orders::OrderFilter { kind: Some("transport".into()), resource: Some(ore.id), ..Default::default() }).await.unwrap();
    assert_eq!(transport.len(), 1);
    assert_eq!(transport[0].origin_village_id, Some(id(V1)));
    assert_eq!(transport[0].destination_village_id, id(V3));
}

#[sqlx::test(migrations = "./migrations")]
async fn cancelled_missions_are_retried_then_fail(pool: PgPool) {
    let (st, fake) = setup(pool, 1_000_000).await;
    simulation::tick(&st, true).await.unwrap();
    let first = legs(&st).await.into_iter().find(|l| l.seq == 0 && l.to_village_id == id(V2)).unwrap();
    fake.set_mission_status(first.mission_id.unwrap(), "expired");
    let r = simulation::tick(&st, true).await.unwrap();
    assert_eq!(r.legs_retried, 1);
    let retried = legs(&st).await.into_iter().find(|l| l.id == first.id).unwrap();
    assert_eq!(retried.status, "published");
    assert_eq!(retried.hold_id, first.hold_id);
    assert_ne!(retried.mission_id, first.mission_id);
    fake.set_mission_status(retried.mission_id.unwrap(), "cancelled");
    let r = simulation::tick(&st, true).await.unwrap();
    assert!(r.orders_failed >= 1);
    assert_eq!(orders::get_order(&st.pool, first.order_id).await.unwrap().unwrap().status, "failed");
    assert!(fake.with(|s| !s.holds[&first.hold_id.unwrap()].active));
}

#[sqlx::test(migrations = "./migrations")]
async fn empty_treasury_mints_then_publishes(pool: PgPool) {
    let (st, fake) = setup(pool, 0).await;
    let r = simulation::tick(&st, true).await.unwrap();
    assert_eq!(r.legs_published, 0);
    assert_eq!(r.minted, 100000);
    assert_eq!(fake.with(|s| s.minted.len()), 1);
    let r = simulation::tick(&st, true).await.unwrap();
    assert!(r.legs_published >= 1, "{r:?}");
}

#[sqlx::test(migrations = "./migrations")]
async fn consumption_drains_the_most_stocked_warehouses(pool: PgPool) {
    let (st, fake) = setup(pool, 1_000_000).await;
    let water_id = catalog::find_resource(&st.pool, Some("water"), None).await.unwrap().unwrap().id;
    fake.set_quantity(Holder::warehouse(id(D3B)), "water", 100);
    world::create_flow(&st.pool, &world::FlowInput { village_id: id(V3), resource_id: water_id, rate_per_hour: -2050.0, source: "population".into(), label: None }).await.unwrap();
    let st = AppState { cfg: Arc::new(Config { sim_time_scale: 1.0, ..config() }), ..st };
    sqlx::query("UPDATE simulation_state SET last_tick_at = now() - interval '1 hour'").execute(&st.pool).await.unwrap();
    simulation::tick(&st, true).await.unwrap();
    // 2050 taken: 2000 from D3A (most stocked) then 50 from D3B.
    assert_eq!(water(&fake, D3A), 0);
    assert_eq!(water(&fake, D3B), 50);
    // One hour later only 50 are left for 2050 wanted: shortage.
    sqlx::query("UPDATE simulation_state SET last_tick_at = now() - interval '1 hour'").execute(&st.pool).await.unwrap();
    let r = simulation::tick(&st, true).await.unwrap();
    assert!(r.shortages >= 1);
    assert_eq!(water(&fake, D3B), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn prices_reflect_scarcity(pool: PgPool) {
    let (st, _fake) = setup(pool, 1_000_000).await;
    simulation::tick(&st, true).await.unwrap();
    let fed = catalog::list_federations(&st.pool).await.unwrap().remove(0);
    let water_id = catalog::find_resource(&st.pool, Some("water"), None).await.unwrap().unwrap().id;
    let local = history::local_prices(&st.pool, fed.id, Some(water_id), None).await.unwrap();
    let at = |v: u128| local.iter().find(|p| p.village_id == id(v)).unwrap().buy_price;
    assert!(at(V1) > at(V3), "water is dearer at the mining village");
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

async fn call(st: &AppState, method: &str, uri: &str, headers: &[(&str, &str)], body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    let res = crate::routes::build(st.clone()).oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

const SERVICE: (&str, &str) = ("x-internal-key", "test-key");

#[sqlx::test(migrations = "./migrations")]
async fn http_villages_and_auth(pool: PgPool) {
    let (st, _) = setup(pool, 1_000_000).await;
    simulation::tick(&st, true).await.unwrap();
    let player = Uuid::new_v4().to_string();
    let admin = [("x-player-id", player.as_str()), ("x-player-roles", "admin")];
    assert_eq!(call(&st, "GET", "/api/villages", &[], None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&st, "GET", "/api/admin/dashboard", &[("x-player-id", &player)], None).await.0, StatusCode::FORBIDDEN);

    let (status, body) = call(&st, "GET", "/api/villages", &[("x-player-id", &player)], None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let city = body["items"].as_array().unwrap().iter().find(|v| v["code"] == "C0003").unwrap().clone();
    assert_eq!(city["warehouseCount"], 2);
    let water_stock = city["stocks"].as_array().unwrap().iter().find(|s| s["targetLevel"] == 500).unwrap();
    assert!(water_stock["quantity"].as_i64().unwrap() > 0, "sum of the city's warehouses");

    let (status, body) = call(&st, "GET", &format!("/api/villages/{}", id(V3)), &admin, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["warehouses"].as_array().unwrap().len(), 2);
    assert_eq!(body["warehouses"][0]["main"], true);

    let (status, body) = call(&st, "GET", &format!("/api/internal/paths?from={}&to={}", id(V1), id(V3)), &[SERVICE], None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hops"].as_array().unwrap().len(), 2);

    let (status, body) = call(&st, "GET", "/api/admin/network", &admin, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["villages"].as_array().unwrap().len(), 3);
    assert_eq!(body["routes"].as_array().unwrap().len(), 2);

    let (status, body) = call(&st, "GET", &format!("/api/admin/stock-history?village={}&resource={}", id(V3), catalog::find_resource(&st.pool, Some("water"), None).await.unwrap().unwrap().id), &admin, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["points"].as_array().unwrap().is_empty());

    // Policy edition: coherence checked.
    let water_id = catalog::find_resource(&st.pool, Some("water"), None).await.unwrap().unwrap().id;
    let (status, _) = call(&st, "PUT", "/api/admin/village-policies", &admin, Some(json!({ "villageId": id(V2), "resourceId": water_id, "minThreshold": 50, "targetLevel": 10 }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = call(&st, "PUT", "/api/admin/village-policies", &admin, Some(json!({ "villageId": id(V2), "resourceId": water_id, "minThreshold": 50, "targetLevel": 90 }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Flow label can be cleared.
    let flow = world::list_flows(&st.pool, Some(id(V1)), None).await.unwrap().remove(0);
    call(&st, "PUT", &format!("/api/admin/flows/{}", flow.id), &admin, Some(json!({ "label": "Miners" }))).await;
    let (_, body) = call(&st, "PUT", &format!("/api/admin/flows/{}", flow.id), &admin, Some(json!({ "label": null }))).await;
    assert_eq!(body["label"], Value::Null);
    // Main warehouse must belong to the village.
    let (status, _) = call(&st, "PUT", &format!("/api/admin/villages/{}", id(V3)), &admin, Some(json!({ "mainWarehouseId": id(D1) }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = call(&st, "PUT", &format!("/api/admin/villages/{}", id(V3)), &admin, Some(json!({ "mainWarehouseId": id(D3B) }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mainWarehouseId"], id(D3B).to_string());
}

#[sqlx::test(migrations = "./migrations")]
async fn warehouse_deliveries(pool: PgPool) {
    let (st, fake) = setup(pool, 1_000_000).await;
    let player = Uuid::new_v4();
    fake.set_quantity(Holder::of("player", player).unwrap(), "water", 2000);
    let body = |key: &str, qty: i64| json!({
        "idempotencyKey": key,
        "warehouse": { "poiUuid": id(D1) },
        "resource": { "code": "water" },
        "quantity": qty,
        "deliverer": { "type": "player", "id": player },
    });
    let (status, res) = call(&st, "POST", "/api/internal/warehouse-deliveries", &[SERVICE], Some(body("k1", 100))).await;
    assert_eq!(status, StatusCode::CREATED, "{res}");
    assert_eq!(res["stock"], 150);
    let (status, _) = call(&st, "POST", "/api/internal/warehouse-deliveries", &[SERVICE], Some(body("k1", 100))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(water(&fake, D1), 150);
    let (status, res) = call(&st, "POST", "/api/internal/warehouse-deliveries", &[SERVICE], Some(body("k1", 5))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(res["error"], "IDEMPOTENCY_CONFLICT");
    // Capacity 1000 per warehouse: only 850 accepted.
    let (_, res) = call(&st, "POST", "/api/internal/warehouse-deliveries", &[SERVICE], Some(body("k2", 1000))).await;
    assert_eq!(res["acceptedQuantity"], 850);
    assert_eq!(res["rejectedQuantity"], 150);
    let (status, res) = call(&st, "POST", "/api/internal/warehouse-deliveries", &[SERVICE], Some(json!({
        "idempotencyKey": "k3", "warehouse": { "id": id(D3B) }, "resource": { "goodType": "water" },
        "quantity": 5000, "deliverer": { "type": "player", "id": player }
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(res["error"], "INSUFFICIENT_STOCK");
}
