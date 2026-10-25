use std::{collections::HashSet, sync::Arc, time::Duration};

use async_trait::async_trait;
use rand::Rng;
use serde_json::{Value, json};
use tokio::time;
use tracing::{debug, info, warn};

use super::{
    Action, ActionContext, ActionTiming, depot_common::*,
    depot_store_reception_to_temp_area::{
        BOX_IN_HANDS_KEY as RECEPTION_BOX_IN_HANDS_KEY,
        RETURN_HOME_KEY as RECEPTION_RETURN_HOME_KEY, counter_box_is_fresh, reception_idle,
    },
};
use crate::{
    error::Result,
    horizon::messages::Vec3,
    npc::{spec::DepotStoreItemConfig, state::NpcStatus},
    slot_layouts::SlotLayouts,
    world::state::ObjectState,
};

/// How often the NPC checks the temp zone while at work.
const CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Depot property mapping `zone_<n>` to the box UUIDs currently sitting in
/// the matching `TempZone/PNJPlace<n>` spot. Kept by the depot itself (its
/// deposit areas watch bodies entering/leaving) — including the removal when
/// a box leaves the temp area, so this action never writes it.
const BOX_TEMP_ZONES_PROP: &str = "pnj_box_temp_zones";

/// Depot property describing the NPC work spots (see the reception action);
/// only the `TempZone` places are read here.
const PNJ_ZONES_PROP: &str = "pnj_zones";
const TEMP_ZONE: &str = "TempZone";

/// Replicated `object_type` of a warehouse shelf. Shelves are parented to the
/// depot; their `position`/`rotation` (radians, yaw in `rotation.y`) are
/// depot-local.
const SHELF_OBJECT_TYPE: &str = "shelf";

/// Replicated `name` of the temp-zone shelf. Same scene and `object_type` as
/// the warehouse shelves, but it holds the staged boxes this action is
/// emptying — never a drop target. Matched as a prefix, so numbered variants
/// (`shelf_cargo_temp_zone2`, …) are excluded too.
const TEMP_ZONE_SHELF_NAME: &str = "shelf_cargo_temp_zone";

/// Geometric fallback for recognizing the temp-zone rack when its `name`
/// does not replicate: a shelf standing within this ground distance (metres)
/// of any `pnj_zones.TempZone` place (its `target` shelf point or rectangle
/// centre, both depot-local like the shelf position) IS the temp rack.
const TEMP_SHELF_EXCLUSION_DIST: f64 = 2.5;

/// Shelf property: one bool per slot, `true` = occupied. Server-maintained
/// (flipped when a box lands in / leaves a slot's detection area). This is
/// the ONLY slot data replicated over the network: the slot geometry itself
/// is static per scene and comes from the [`SlotLayouts`] registry (JSON
/// files exported from Godot), keyed by the shelf's replicated `scenename`.
const SLOTS_OCCUPIED_PROP: &str = "shelf_slots_occupied";

/// Blackboard key holding the UUID of the box the NPC picked up from the temp
/// zone. Deliberately distinct from the reception action's key so an
/// interrupted carry resumes in the right flow — and each action refuses to
/// start while the other one's box is in hands.
pub const SHELF_BOX_IN_HANDS_KEY: &str = "depot_store_item_box_in_hands";

/// Metres the NPC stands in front of the shelf face when no per-NPC
/// `stand_distance_m` is configured.
const DEFAULT_STAND_DISTANCE: f64 = 1.0;

/// How many shelf/slot re-picks one carry attempts before giving up until a
/// later run (a slot can be filled by someone else during the walk).
const MAX_SLOT_RETRIES: usize = 3;

/// How long to wait for the server's `npc_arrived` notice before attempting
/// the grab/drop from wherever the NPC stopped.
const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Pause between arriving at the temp place and grabbing the box.
const SETTLE_BEFORE_PICKUP: Duration = Duration::from_secs(2);

/// Pause between arriving in front of the shelf and dropping the box.
const SETTLE_BEFORE_DROP: Duration = Duration::from_secs(2);

/// How long to let the server finish an `npc_face_position` pivot before
/// aiming the head (worst-case half turn at half a turn per second, plus
/// replication margin).
const TURN_WAIT: Duration = Duration::from_millis(1300);

/// Pause between facing/aiming and pressing the action key.
const PAUSE_BEFORE_ACTION: Duration = Duration::from_millis(500);

/// How long to wait for the grab/drop to replicate back as a reparent.
const GRAB_CONFIRM_TIMEOUT: Duration = Duration::from_secs(4);
const DROP_CONFIRM_TIMEOUT: Duration = Duration::from_secs(4);

/// While the NPC is [`AtWork`](NpcStatus::AtWork), moves boxes from the
/// depot's temp zone onto the warehouse shelves — the step after
/// `depot_store_reception_to_temp_area`.
///
/// Whenever the depot's `pnj_box_temp_zones` lists a box that is replicated
/// into view and not in anyone's hands, the NPC first makes sure some shelf
/// has a free slot (a box grabbed with nowhere to put it down would stay in
/// its hands forever), then picks the box up exactly like the reception
/// pickup: walk to the matching `pnj_zones.TempZone` place, settle, face the
/// box, aim the head, press the action key, and only count the grab once the
/// box replicates as reparented under the NPC ([`SHELF_BOX_IN_HANDS_KEY`]).
///
/// The carry then targets the nearest usable shelf: `object_type` `"shelf"`,
/// parented to the depot, whose `scenename` has an exported slot layout in
/// the [`SlotLayouts`] registry matching the length of
/// [`SLOTS_OCCUPIED_PROP`], with at least one free entry (lowest free index
/// first — the layouts order slots bottom row first). The drop point and the
/// standing point are both computed from the shelf's depot-local transform:
/// the slot's layout position rotated by the shelf yaw, and the same point
/// pulled `stand_distance` metres along the layout's `front` vector at floor
/// height — so the NPC stands laterally aligned with the slot's column
/// whatever the shelf's orientation. After the walk the slot is re-checked
/// (someone may have filled it) and the shelf/slot re-picked up to
/// [`MAX_SLOT_RETRIES`] times. The drop only counts once the box replicates
/// as reparented off the NPC; otherwise the carry retries on a later run.
pub struct DepotStoreItem {
    /// UUID of the depot the NPC works at, taken from `place_where_work`.
    /// `None` means the NPC has no workplace, so this action never runs.
    work_uuid: Option<String>,
    /// Metres the NPC stands in front of the shelf face.
    stand_distance: f64,
    /// Slot geometry per scene, loaded from the exported JSON files.
    layouts: Arc<SlotLayouts>,
}

impl DepotStoreItem {
    pub fn new(work_uuid: Option<String>, config: Option<DepotStoreItemConfig>) -> Self {
        Self::with_layouts(work_uuid, config, SlotLayouts::shared())
    }

    /// Like [`new`](Self::new), with an explicit layout registry (tests).
    pub fn with_layouts(
        work_uuid: Option<String>,
        config: Option<DepotStoreItemConfig>,
        layouts: Arc<SlotLayouts>,
    ) -> Self {
        Self {
            work_uuid,
            stand_distance: config
                .and_then(|c| c.stand_distance_m)
                .unwrap_or(DEFAULT_STAND_DISTANCE),
            layouts,
        }
    }

    /// A box waiting in the temp zone that the NPC could grab right now:
    /// listed in `pnj_box_temp_zones`, replicated into view, and not riding
    /// in anyone's hands. Returns `(box uuid, TempZone place name)`; `None`
    /// when the depot, the zone description, or such a box is not in view.
    fn takeable_temp_box(&self, ctx: &ActionContext) -> Option<(String, String)> {
        let uuid = self.work_uuid.as_ref()?;
        let (places, occupied) = {
            let depot = ctx.world.objects.get(uuid)?;
            (
                depot.property(PNJ_ZONES_PROP)?.get(TEMP_ZONE)?.as_object()?.clone(),
                depot.property(BOX_TEMP_ZONES_PROP)?.clone(),
            )
        };
        for name in places.keys() {
            let Some(key) = zone_key(name) else { continue };
            let Some(boxes) = occupied.get(&key).and_then(Value::as_array) else {
                continue;
            };
            for item in boxes.iter().filter_map(Value::as_str) {
                if ctx.world.objects.contains_key(item) && !carried_by_player(ctx, item) {
                    return Some((item.to_string(), name.clone()));
                }
            }
        }
        None
    }

    /// Random standing point inside the named `TempZone` place rectangle,
    /// depot-local (same contract as the reception standing spot).
    fn place_stand_point(&self, ctx: &mut ActionContext, place: &str) -> Option<Vec3> {
        let uuid = self.work_uuid.as_ref()?;
        let (center, size) = {
            let depot = ctx.world.objects.get(uuid)?;
            let p = depot
                .property(PNJ_ZONES_PROP)?
                .get(TEMP_ZONE)?
                .get(place)?
                .clone();
            (vec3_from(p.get("position")?)?, vec3_from(p.get("size")?)?)
        };
        Some(Vec3 {
            x: center.x + ctx.rng.gen_range(-size.x / 2.0..=size.x / 2.0),
            y: center.y,
            z: center.z + ctx.rng.gen_range(-size.z / 2.0..=size.z / 2.0),
        })
    }

    /// Nearest usable shelf with a free slot, as `(shelf uuid, slot index)`.
    /// Usable: `object_type` `"shelf"`, parented to the depot, not the
    /// temp-zone shelf ([`TEMP_ZONE_SHELF_NAME`]), not in `exclude`, whose
    /// scene has an exported slot layout index-matched with the occupancy
    /// array. The slot is the lowest free index (bottom row fills first per
    /// the layouts); the shelf is the closest one on the ground plane —
    /// unresolved positions sort last, so replication hiccups only cost
    /// walking distance, never correctness.
    fn select_shelf(&self, ctx: &ActionContext, exclude: &HashSet<String>) -> Option<(String, usize)> {
        let depot = self.work_uuid.as_ref()?;
        let temp_points = self.temp_zone_points(ctx);
        // Per-reason rejection counters, logged when nothing is usable so a
        // "no usable shelf" in the wild diagnoses itself.
        let (mut other_parent, mut temp_zone, mut no_occupancy) = (0usize, 0usize, 0usize);
        let (mut no_layout, mut slot_mismatch, mut full) = (0usize, 0usize, 0usize);
        let mut shelves_in_view = 0usize;
        // Collect before resolving positions: `resolve_global` reads the same
        // objects map an iterator would keep locked.
        let mut candidates: Vec<(String, usize, Vec3)> = Vec::new();
        for entry in ctx.world.objects.iter() {
            let shelf = entry.value();
            if shelf.object_type != SHELF_OBJECT_TYPE {
                continue;
            }
            shelves_in_view += 1;
            if shelf.parent_id.as_deref() != Some(depot.as_str()) {
                other_parent += 1;
                debug!(npc = %ctx.npc_name, shelf = %shelf.uuid, parent = ?shelf.parent_id, "shelf skipped: parented outside the depot");
                continue;
            }
            if exclude.contains(&shelf.uuid) {
                continue;
            }
            // The temp-zone shelf is where the box comes FROM: storing it
            // back there would loop forever.
            if is_temp_zone_shelf(shelf, &temp_points) {
                temp_zone += 1;
                debug!(npc = %ctx.npc_name, shelf = %shelf.uuid, "shelf skipped: the temp-zone rack");
                continue;
            }
            let Some(occupied) = shelf.property(SLOTS_OCCUPIED_PROP).and_then(Value::as_array)
            else {
                no_occupancy += 1;
                debug!(npc = %ctx.npc_name, shelf = %shelf.uuid, "shelf skipped: no shelf_slots_occupied array replicated");
                continue;
            };
            // Without an exported slot layout for this scene (or with a
            // mismatched slot count) the shelf is not usable yet — never an
            // error, the layout file may simply not be shipped.
            match shelf.scenename.as_deref().and_then(|s| self.layouts.get(s)) {
                None => {
                    no_layout += 1;
                    debug!(npc = %ctx.npc_name, shelf = %shelf.uuid, scenename = ?shelf.scenename, "shelf skipped: no slot layout exported for its scene");
                    continue;
                }
                Some(l) if l.slot_count() != occupied.len() => {
                    slot_mismatch += 1;
                    debug!(npc = %ctx.npc_name, shelf = %shelf.uuid, layout_slots = l.slot_count(), occupied_len = occupied.len(), "shelf skipped: occupancy length disagrees with the layout");
                    continue;
                }
                Some(_) => {}
            }
            let Some(idx) = occupied.iter().position(|s| s.as_bool() == Some(false)) else {
                full += 1;
                continue;
            };
            candidates.push((shelf.uuid.clone(), idx, shelf.position));
        }
        if candidates.is_empty() {
            warn!(
                npc = %ctx.npc_name,
                objects_in_view = ctx.world.objects.len(),
                shelves_in_view,
                other_parent,
                temp_zone,
                no_occupancy,
                no_layout,
                slot_mismatch,
                full,
                already_tried = exclude.len(),
                layouts_loaded = self.layouts.len(),
                "no usable shelf: per-reason rejection counts (RUST_LOG=npc=debug for per-shelf details)"
            );
        }
        let my_pos = ctx.world.my_global_position();
        candidates
            .into_iter()
            .map(|(uuid, idx, pos)| {
                let dist = match (my_pos, ctx.world.resolve_global(pos, Some(depot))) {
                    (Some(me), Some(global)) => (global.x - me.x).hypot(global.z - me.z),
                    _ => f64::INFINITY,
                };
                (dist, uuid, idx)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, uuid, idx)| (uuid, idx))
    }

    /// Depot-local points of every `pnj_zones.TempZone` place — the `target`
    /// shelf points and the rectangle centres — used to recognize the
    /// temp-zone rack geometrically when its `name` is not replicated.
    fn temp_zone_points(&self, ctx: &ActionContext) -> Vec<Vec3> {
        let Some(uuid) = self.work_uuid.as_ref() else {
            return Vec::new();
        };
        let Some(depot) = ctx.world.objects.get(uuid) else {
            return Vec::new();
        };
        let Some(places) = depot
            .property(PNJ_ZONES_PROP)
            .and_then(|z| z.get(TEMP_ZONE))
            .and_then(Value::as_object)
        else {
            return Vec::new();
        };
        places
            .values()
            .flat_map(|p| [p.get("target"), p.get("position")])
            .flatten()
            .filter_map(vec3_from)
            .collect()
    }

    /// Depot-local geometry for one shelf slot: the slot point itself (to
    /// face and aim at) and the standing point in front of it — the slot's
    /// column pulled `stand_distance` metres along the layout's `front`
    /// vector, at the shelf's floor height. `None` when the shelf, its scene
    /// layout, or the slot index is unknown.
    fn slot_points(&self, ctx: &ActionContext, shelf: &str, idx: usize) -> Option<(Vec3, Vec3)> {
        let (base, yaw, slot_local, front) = {
            let s = ctx.world.objects.get(shelf)?;
            let layout = self.layouts.get(s.scenename.as_deref()?)?;
            (s.position, s.rotation.y, layout.slot(idx)?, layout.front_vec())
        };
        let slot = rotate_y(slot_local, yaw);
        let stand = rotate_y(
            Vec3 {
                x: slot_local.x + front.x * self.stand_distance,
                y: 0.0,
                z: slot_local.z + front.z * self.stand_distance,
            },
            yaw,
        );
        Some((
            Vec3 { x: base.x + slot.x, y: base.y + slot.y, z: base.z + slot.z },
            Vec3 { x: base.x + stand.x, y: base.y + stand.y, z: base.z + stand.z },
        ))
    }

    /// Whether slot `idx` of `shelf` still reads as free — re-checked after
    /// the walk, since anyone may have filled it meanwhile.
    fn slot_still_free(&self, ctx: &ActionContext, shelf: &str, idx: usize) -> bool {
        let Some(s) = ctx.world.objects.get(shelf) else {
            return false;
        };
        s.property(SLOTS_OCCUPIED_PROP)
            .and_then(Value::as_array)
            .and_then(|a| a.get(idx))
            .and_then(Value::as_bool)
            == Some(false)
    }

    /// Walk to `stand` (local to the depot), wait for the arrival notice (or
    /// reach the spot per our own position), settle, face the box, aim the
    /// head, and press the action key on it. The pickup only counts once the
    /// server reparents the box under the NPC — then
    /// [`SHELF_BOX_IN_HANDS_KEY`] is set; otherwise everything is left
    /// untouched and a later run retries from scratch.
    async fn pick_up_from_temp(
        &self,
        ctx: &mut ActionContext,
        item: &str,
        stand: Vec3,
    ) -> Result<()> {
        let Some(depot) = self.work_uuid.clone() else {
            return Ok(());
        };
        // A stale arrival flag from a previous trip must not fake this one.
        ctx.world.clear_arrived();
        info!(npc = %ctx.npc_name, item = %item, "walking to the temp-zone place");
        ctx.send
            .send(
                "player",
                "client_action",
                json!({
                    "action": "npc_go_to_position",
                    "uuid": depot,
                    "position": stand,
                }),
            )
            .await?;

        match wait_for_arrival(ctx, &depot, stand, ARRIVAL_TIMEOUT).await {
            ArrivalOutcome::Notified => {
                info!(npc = %ctx.npc_name, item = %item, "arrived at the temp-zone place");
                ctx.send
                    .send(
                        "player",
                        "client_action",
                        json!({
                            "action": "npc_arrived_ack",
                            "uuid": depot,
                        }),
                    )
                    .await?;
                ctx.world.clear_arrived();
            }
            ArrivalOutcome::CloseEnough => {
                info!(npc = %ctx.npc_name, item = %item, "close enough to the temp-zone place (no server arrival notice)");
            }
            ArrivalOutcome::TimedOut => {
                let dist = ground_distance_to(ctx, &depot, stand)
                    .map(|d| format!("{d:.2}"))
                    .unwrap_or_else(|| "?".into());
                warn!(
                    npc = %ctx.npc_name,
                    item = %item,
                    distance_to_target_m = %dist,
                    "no arrival notice; attempting the grab from here"
                );
            }
        }

        time::sleep(SETTLE_BEFORE_PICKUP).await;

        // Someone may have grabbed the box during the walk/settle: a carried
        // box cannot be grabbed, so retry on a later run instead of pressing
        // the action key on their hands. Nothing was consumed.
        if carried_by_player(ctx, item) {
            warn!(npc = %ctx.npc_name, item = %item, "box taken during the walk; abandoning the pickup for a retry");
            return Ok(());
        }

        // Face the box first (the server pivots the body smoothly around its
        // own up axis), then aim the head so the interaction ray matches.
        info!(npc = %ctx.npc_name, item = %item, "turning to face the box");
        ctx.send
            .send(
                "player",
                "client_action",
                json!({
                    "action": "npc_face_position",
                    "uuid": item,
                    "position": Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                }),
            )
            .await?;
        time::sleep(TURN_WAIT).await;

        match aim_at(ctx, item) {
            Some((pitch, eye_dist)) => {
                info!(
                    npc = %ctx.npc_name,
                    item = %item,
                    pitch = format!("{pitch:.3}"),
                    eye_dist_m = format!("{eye_dist:.2}"),
                    "aiming head at the box"
                );
                if eye_dist > 2.5 {
                    warn!(npc = %ctx.npc_name, item = %item, "standing too far from the box; the grab will likely fail");
                }
                ctx.send
                    .send(
                        "player",
                        "client_action",
                        json!({
                            "action": "update_property",
                            "head": pitch,
                        }),
                    )
                    .await?;
            }
            None => {
                warn!(npc = %ctx.npc_name, item = %item, "positions unresolved; picking up without aiming the head");
            }
        }

        time::sleep(PAUSE_BEFORE_ACTION).await;

        info!(npc = %ctx.npc_name, item = %item, "picking up the box");
        ctx.send
            .send(
                "player",
                "client_action",
                json!({
                    "action": "action",
                    "target_uuid": item,
                }),
            )
            .await?;

        if wait_for_box_in_hands(ctx, item, GRAB_CONFIRM_TIMEOUT).await {
            info!(npc = %ctx.npc_name, item = %item, "box in hands");
            ctx.state.set(SHELF_BOX_IN_HANDS_KEY, json!(item));
        } else {
            warn!(npc = %ctx.npc_name, item = %item, "grab not confirmed by the server; abandoning for a retry");
        }
        Ok(())
    }

    /// Carry the box in hands to the nearest usable shelf and drop it into
    /// its lowest free slot: walk to the computed standing point, wait for
    /// arrival, settle, re-check the slot, face and aim at the slot point,
    /// and press the action key — which releases a carried item. The drop
    /// only counts once the box replicates as reparented off the NPC;
    /// otherwise [`SHELF_BOX_IN_HANDS_KEY`] stays set and the carry retries
    /// on a later run. The depot updates its own `pnj_box_temp_zones` when
    /// the box left the temp area, and the server flips
    /// `shelf_slots_occupied` when the box lands in the slot.
    async fn store_on_shelf(&self, ctx: &mut ActionContext, item: &str) -> Result<()> {
        let Some(depot) = self.work_uuid.clone() else {
            return Ok(());
        };
        // The carry may have been lost on the way (e.g. the server auto-drops
        // an out-of-reach item): with nothing in hands the action key would
        // GRAB, not release, so re-arm the whole flow instead.
        let held = ctx
            .world
            .objects
            .get(item)
            .and_then(|o| o.parent_id.clone())
            .is_some_and(|p| p == ctx.world.my_uuid);
        if !held {
            warn!(npc = %ctx.npc_name, item = %item, "box no longer in hands; abandoning the carry");
            ctx.state.remove(SHELF_BOX_IN_HANDS_KEY);
            return Ok(());
        }

        // Shelves whose slot data failed to parse during THIS carry — skipped
        // on re-picks rather than looped over forever.
        let mut skipped: HashSet<String> = HashSet::new();
        for _ in 0..MAX_SLOT_RETRIES {
            // Every usable shelf full (or none published yet): keep the box
            // and retry on a later run.
            let Some((shelf, idx)) = self.select_shelf(ctx, &skipped) else {
                warn!(npc = %ctx.npc_name, item = %item, "no usable shelf with a free slot; retrying later");
                return Ok(());
            };
            let Some((slot, stand)) = self.slot_points(ctx, &shelf, idx) else {
                warn!(npc = %ctx.npc_name, item = %item, shelf = %shelf, slot = idx, "slot position unparseable; trying another shelf");
                skipped.insert(shelf);
                continue;
            };

            ctx.world.clear_arrived();
            info!(npc = %ctx.npc_name, item = %item, shelf = %shelf, slot = idx, "carrying the box to the shelf");
            ctx.send
                .send(
                    "player",
                    "client_action",
                    json!({
                        "action": "npc_go_to_position",
                        "uuid": depot,
                        "position": stand,
                    }),
                )
                .await?;

            match wait_for_arrival(ctx, &depot, stand, ARRIVAL_TIMEOUT).await {
                ArrivalOutcome::Notified => {
                    info!(npc = %ctx.npc_name, item = %item, shelf = %shelf, "arrived in front of the shelf");
                    ctx.send
                        .send(
                            "player",
                            "client_action",
                            json!({
                                "action": "npc_arrived_ack",
                                "uuid": depot,
                            }),
                        )
                        .await?;
                    ctx.world.clear_arrived();
                }
                ArrivalOutcome::CloseEnough => {
                    info!(npc = %ctx.npc_name, item = %item, shelf = %shelf, "close enough to the shelf (no server arrival notice)");
                }
                ArrivalOutcome::TimedOut => {
                    let dist = ground_distance_to(ctx, &depot, stand)
                        .map(|d| format!("{d:.2}"))
                        .unwrap_or_else(|| "?".into());
                    warn!(
                        npc = %ctx.npc_name,
                        item = %item,
                        distance_to_target_m = %dist,
                        "no arrival notice; attempting the drop from here"
                    );
                }
            }

            time::sleep(SETTLE_BEFORE_DROP).await;

            // Someone may have filled the slot during the walk: re-pick
            // rather than dropping a second box into it.
            if !self.slot_still_free(ctx, &shelf, idx) {
                info!(npc = %ctx.npc_name, item = %item, shelf = %shelf, slot = idx, "slot filled during the walk; picking another");
                continue;
            }

            // Face the slot, then aim the head at it so the carried box —
            // which follows the gaze — hangs toward the slot when released.
            info!(npc = %ctx.npc_name, item = %item, shelf = %shelf, slot = idx, "turning to face the slot");
            ctx.send
                .send(
                    "player",
                    "client_action",
                    json!({
                        "action": "npc_face_position",
                        "uuid": depot,
                        "position": slot,
                    }),
                )
                .await?;
            time::sleep(TURN_WAIT).await;

            match aim_at_local(ctx, &depot, slot) {
                Some((pitch, eye_dist)) => {
                    info!(
                        npc = %ctx.npc_name,
                        item = %item,
                        pitch = format!("{pitch:.3}"),
                        eye_dist_m = format!("{eye_dist:.2}"),
                        "aiming head at the slot"
                    );
                    // Nav arrival forgives up to 1.5 m: surface a stand point
                    // left too far from the slot — the released box would
                    // fall short of it.
                    if eye_dist > 2.5 {
                        warn!(npc = %ctx.npc_name, item = %item, shelf = %shelf, "standing too far from the slot; the drop will likely miss");
                    }
                    ctx.send
                        .send(
                            "player",
                            "client_action",
                            json!({
                                "action": "update_property",
                                "head": pitch,
                            }),
                        )
                        .await?;
                }
                None => {
                    warn!(npc = %ctx.npc_name, item = %item, "positions unresolved; dropping without aiming the head");
                }
            }

            time::sleep(PAUSE_BEFORE_ACTION).await;

            info!(npc = %ctx.npc_name, item = %item, shelf = %shelf, slot = idx, "dropping the box into the slot");
            ctx.send
                .send(
                    "player",
                    "client_action",
                    json!({
                        "action": "action",
                        "target_uuid": item,
                    }),
                )
                .await?;

            if wait_for_box_released(ctx, item, DROP_CONFIRM_TIMEOUT).await {
                info!(npc = %ctx.npc_name, item = %item, shelf = %shelf, slot = idx, "box stored on the shelf");
                ctx.state.remove(SHELF_BOX_IN_HANDS_KEY);
            } else {
                warn!(npc = %ctx.npc_name, item = %item, "drop not confirmed by the server; retrying later");
            }
            return Ok(());
        }
        warn!(npc = %ctx.npc_name, item = %item, "no slot survived the re-picks; retrying later");
        Ok(())
    }
}

/// Whether `shelf` is the temp-zone rack — never a drop target. Recognized
/// by its replicated `name` (a [`TEMP_ZONE_SHELF_NAME`] prefix, looked up
/// both at the payload top level and nested under `object_data`, since the
/// pipeline is not consistent about where it travels), or by standing next
/// to a `TempZone` place ([`TEMP_SHELF_EXCLUSION_DIST`] on the ground plane,
/// depot-local frames on both sides) — the geometric check also covers a
/// name that never replicated or replicated differently than expected.
fn is_temp_zone_shelf(shelf: &ObjectState, temp_points: &[Vec3]) -> bool {
    let named_temp = shelf
        .property("name")
        .or_else(|| shelf.property("object_data").and_then(|o| o.get("name")))
        .and_then(Value::as_str)
        .is_some_and(|n| n.starts_with(TEMP_ZONE_SHELF_NAME));
    named_temp
        || temp_points.iter().any(|t| {
            (t.x - shelf.position.x).hypot(t.z - shelf.position.z) <= TEMP_SHELF_EXCLUSION_DIST
        })
}

/// UUID of the box currently in the NPC's hands, per the blackboard.
fn box_in_hands(ctx: &ActionContext) -> Option<String> {
    ctx.state
        .get(SHELF_BOX_IN_HANDS_KEY)
        .and_then(|v| v.as_str().map(str::to_string))
}

#[async_trait]
impl Action for DepotStoreItem {
    fn name(&self) -> &'static str {
        "depot_store_item"
    }

    fn priority(&self) -> i32 {
        // Just below depot_store_reception_to_temp_area (95): a fresh deposit
        // on the counter — or its own pending carry — always wins over
        // tidying the temp zone.
        94
    }

    fn timing(&self) -> ActionTiming {
        // Poll the temp zone at ~0.5 Hz rather than on every scheduler tick.
        ActionTiming {
            min_interval: Some(CHECK_INTERVAL),
            ..Default::default()
        }
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        if ctx.state.status() != NpcStatus::AtWork {
            return false;
        }
        // Hands busy with the reception carry: its action finishes first.
        if ctx.state.get(RECEPTION_BOX_IN_HANDS_KEY).is_some() {
            return false;
        }
        // A box already in hands must reach a shelf regardless of the zone.
        if box_in_hands(ctx).is_some() {
            return true;
        }
        // Reception role split: the reception owner only HELPS stock when it has
        // nothing to receive — no waiting player, no counter box — and is not
        // mid walk-back to its reception spot. Stockers and uncoordinated NPCs
        // stock as usual. (Placed after the in-hands check so a role flip while
        // carrying a shelf box never strands it.)
        if let Some(depot) = &self.work_uuid {
            match ctx.depot_role {
                crate::depot::DepotRole::Reception => {
                    if ctx.state.get(RECEPTION_RETURN_HOME_KEY).is_some() {
                        return false;
                    }
                    if !reception_idle(ctx, depot) {
                        return false;
                    }
                }
                // A box that just landed on the counter is the reception NPC's
                // to take, but the stocker must not disappear into a shelf carry
                // meanwhile: if the reception NPC does not collect it, the
                // stocker takes over as soon as the grace passes
                // (`depot_store_reception_to_temp_area`, priority 95), and the
                // box would otherwise wait out a whole carry first.
                crate::depot::DepotRole::Stocker => {
                    if counter_box_is_fresh(ctx, depot) {
                        return false;
                    }
                }
                crate::depot::DepotRole::Uncoordinated => {}
            }
        }
        // Shelf availability is deliberately NOT checked here (can_run is
        // polled every tick); `run` checks it before grabbing anything.
        self.takeable_temp_box(ctx).is_some()
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        // Finish carrying a picked-up box before looking at the zone again.
        if let Some(item) = box_in_hands(ctx) {
            return self.store_on_shelf(ctx, &item).await;
        }
        let Some((item, place)) = self.takeable_temp_box(ctx) else {
            return Ok(());
        };
        // Check the shelves BEFORE picking the box up: grabbed with nowhere
        // to put it down, it would stay in the NPC's hands forever.
        if self.select_shelf(ctx, &HashSet::new()).is_none() {
            warn!(npc = %ctx.npc_name, item = %item, "no usable shelf with a free slot; leaving the box in the temp zone");
            return Ok(());
        }
        // Zone data can lag behind the depot's first frame; retry later
        // rather than walking to an unknown spot.
        let Some(stand) = self.place_stand_point(ctx, &place) else {
            warn!(npc = %ctx.npc_name, place = %place, "temp-zone place unparseable; retrying");
            return Ok(());
        };
        self.pick_up_from_temp(ctx, &item, stand).await?;
        // Grab confirmed: carry it to the shelf in the same run rather than
        // waiting a scheduler round-trip with a box dangling in hands.
        if box_in_hands(ctx).is_some() {
            return self.store_on_shelf(ctx, &item).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use rand::{SeedableRng, rngs::StdRng};

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind},
        npc::state::NpcState,
        slot_layouts::SlotLayout,
        world::view::WorldView,
    };

    const DEPOT: &str = "a9340169-767c-4b02-a2f3-67f2786375b3";
    const ITEM: &str = "11111111-1111-1111-1111-111111111111";
    const SHELF1: &str = "c76cb110-70db-4e5e-9dd8-000000000001";
    const SHELF2: &str = "c76cb110-70db-4e5e-9dd8-000000000002";
    const SCENE: &str = "scenes/_universe/props/furniture/metal_shelf.tscn";

    // The TempZone place used by `depot_frame`.
    const TEMP_SIZE: (f64, f64, f64) = (0.37, 0.5, 0.24);
    const TEMP0_CENTER: (f64, f64, f64) = (-4.5, 0.251, 9.471);

    /// A registry holding a 2-column × 2-level grid for the shelf scene,
    /// local to the shelf (front face = +Z).
    fn test_layouts() -> Arc<SlotLayouts> {
        let layout = SlotLayout {
            schema_version: 1,
            scenename: Some(SCENE.into()),
            object_type: Some("shelf".into()),
            front: [0.0, 0.0, 1.0],
            slots: vec![
                [-0.5, 0.35, 0.4],
                [0.5, 0.35, 0.4],
                [-0.5, 0.8, 0.4],
                [0.5, 0.8, 0.4],
            ],
        };
        Arc::new(SlotLayouts::from_map(HashMap::from([(SCENE.to_string(), layout)])))
    }

    fn all_free() -> Value {
        json!([false, false, false, false])
    }
    fn all_full() -> Value {
        json!([true, true, true, true])
    }

    fn dummy_ctx(world: Arc<WorldView>) -> (ActionContext, tokio::sync::mpsc::Receiver<Value>) {
        // Wide enough for a full pickup + carry exchange — a full channel
        // would deadlock a paused-time test (send blocks, nothing drains).
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let ctx = ActionContext {
            npc_name: "Test".into(),
            world,
            state: Arc::new(NpcState::new()),
            send: HorizonSender::__test_new(tx),
            rng: StdRng::seed_from_u64(0),
            allowed_actions: None,
            depot_role: crate::depot::DepotRole::Uncoordinated,
        };
        (ctx, rx)
    }

    /// The NPC's depot with one TempZone place and the given occupancy map.
    fn depot_frame(box_zones: Value) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 6,
            object_id: DEPOT.into(),
            object_type: "depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "position": {"x": 0.0, "y": 0.0, "z": 0.0},
                "pnj_box_temp_zones": box_zones,
                "pnj_zones": {
                    "TempZone": {
                        "PNJPlace0": {
                            "path": "TempZone/Place0/PNJPlace0",
                            "position": [TEMP0_CENTER.0, TEMP0_CENTER.1, TEMP0_CENTER.2],
                            "size": [TEMP_SIZE.0, TEMP_SIZE.1, TEMP_SIZE.2],
                            "target": [-5.354, 0.993, 9.46],
                        },
                    },
                },
            }),
        }
    }

    /// A shelf parented to the depot, with the given occupancy.
    fn shelf_frame(uuid: &str, pos: (f64, f64, f64), yaw: f64, occupied: Value) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 6,
            object_id: uuid.into(),
            object_type: "shelf".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "parent_id": DEPOT,
                "position": {"x": pos.0, "y": pos.1, "z": pos.2},
                "rotation": {"x": 0.0, "y": yaw, "z": 0.0},
                "scenename": SCENE,
                "shelf_slots_occupied": occupied,
            }),
        }
    }

    /// A box entering the NPC's view.
    fn item_frame(uuid: &str, pos: (f64, f64, f64)) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 6,
            object_id: uuid.into(),
            object_type: "item".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "position": {"x": pos.0, "y": pos.1, "z": pos.2},
                "scenename": "hauling_box.tscn",
            }),
        }
    }

    /// Another player entering the NPC's view.
    fn player_frame(uuid: &str) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: uuid.into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "position": {"x": 0.0, "y": 0.0, "z": 2.0},
            }),
        }
    }

    /// A frame reparenting an item (grabbed / released).
    fn reparent_frame(uuid: &str, parent: &str) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::UpdateProperty,
            channel: 0,
            object_id: uuid.into(),
            object_type: "item".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "parent_id": parent }),
        }
    }

    /// A self frame placing the NPC in the world.
    fn self_frame(uuid: &str, pos: (f64, f64, f64)) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::ZoneEnter,
            channel: 0,
            object_id: uuid.into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({
                "position": {"x": pos.0, "y": pos.1, "z": pos.2},
            }),
        }
    }

    /// A self frame carrying the server's `npc_arrived` notice.
    fn arrived_frame(uuid: &str) -> GorcFrame {
        GorcFrame {
            kind: GorcKind::Update,
            channel: 0,
            object_id: uuid.into(),
            object_type: "player".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "npc_arrived": true }),
        }
    }

    /// Pop every message queued on the send channel.
    fn drain(rx: &mut tokio::sync::mpsc::Receiver<Value>) -> Vec<Value> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn action() -> DepotStoreItem {
        DepotStoreItem::with_layouts(Some(DEPOT.to_string()), None, test_layouts())
    }

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn slot_and_stand_points_at_zero_yaw() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        world.apply(&shelf_frame(SHELF1, (10.0, 0.0, 5.0), 0.0, all_free()));
        let (ctx, _rx) = dummy_ctx(world);

        let (slot, stand) = action().slot_points(&ctx, SHELF1, 0).unwrap();
        approx(slot.x, 9.5);
        approx(slot.y, 0.35);
        approx(slot.z, 5.4);
        // Standing point: same column, shelf floor height, 1 m out of the
        // front face (+Z local).
        approx(stand.x, 9.5);
        approx(stand.y, 0.0);
        approx(stand.z, 6.4);
    }

    #[test]
    fn slot_and_stand_points_follow_the_shelf_yaw() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        world.apply(&shelf_frame(
            SHELF1,
            (10.0, 0.0, 5.0),
            std::f64::consts::FRAC_PI_2,
            all_free(),
        ));
        let (ctx, _rx) = dummy_ctx(world);

        // rotate_y((-0.5, 0.35, 0.4), π/2) = (0.4, 0.35, 0.5) — the front
        // face now points toward +X in the depot frame.
        let (slot, stand) = action().slot_points(&ctx, SHELF1, 0).unwrap();
        approx(slot.x, 10.4);
        approx(slot.y, 0.35);
        approx(slot.z, 5.5);
        approx(stand.x, 11.4);
        approx(stand.y, 0.0);
        approx(stand.z, 5.5);
    }

    #[test]
    fn stand_distance_comes_from_the_config() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        world.apply(&shelf_frame(SHELF1, (0.0, 0.0, 0.0), 0.0, all_free()));
        let (ctx, _rx) = dummy_ctx(world);

        let a = DepotStoreItem::with_layouts(
            Some(DEPOT.to_string()),
            Some(DepotStoreItemConfig { stand_distance_m: Some(2.5) }),
            test_layouts(),
        );
        let (_, stand) = a.slot_points(&ctx, SHELF1, 0).unwrap();
        approx(stand.z, 0.4 + 2.5);
    }

    #[test]
    fn nearest_shelf_with_a_free_slot_wins() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!({})));
        world.apply(&shelf_frame(SHELF1, (5.0, 0.0, 0.0), 0.0, all_free()));
        world.apply(&shelf_frame(SHELF2, (20.0, 0.0, 0.0), 0.0, all_free()));
        let (ctx, _rx) = dummy_ctx(Arc::clone(&world));

        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), Some((SHELF1.into(), 0)));

        // The near shelf fills up: the far one is picked instead.
        world.apply(&shelf_frame(SHELF1, (5.0, 0.0, 0.0), 0.0, all_full()));
        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), Some((SHELF2.into(), 0)));
    }

    #[test]
    fn lowest_free_slot_index_is_chosen() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        world.apply(&shelf_frame(
            SHELF1,
            (5.0, 0.0, 0.0),
            0.0,
            json!([true, true, false, false]),
        ));
        let (ctx, _rx) = dummy_ctx(world);
        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), Some((SHELF1.into(), 2)));
    }

    #[test]
    fn shelf_without_usable_slot_data_is_skipped() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        // A scene no layout file was exported for.
        let mut frame = shelf_frame(SHELF1, (5.0, 0.0, 0.0), 0.0, all_free());
        frame.data["scenename"] = json!("scenes/props/unknown_rack.tscn");
        world.apply(&frame);
        // Occupancy length disagreeing with the 4-slot layout.
        world.apply(&shelf_frame(SHELF2, (6.0, 0.0, 0.0), 0.0, json!([false, false, false])));
        let (ctx, _rx) = dummy_ctx(world);
        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), None);
    }

    #[test]
    fn temp_zone_shelf_is_never_a_drop_target() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        // The temp-zone rack: same scene, same type, free slots — but it is
        // where the box comes from.
        let mut frame = shelf_frame(SHELF1, (5.0, 0.0, 0.0), 0.0, all_free());
        frame.data["name"] = json!("shelf_cargo_temp_zone");
        world.apply(&frame);
        world.apply(&shelf_frame(SHELF2, (20.0, 0.0, 0.0), 0.0, all_free()));
        let (ctx, _rx) = dummy_ctx(world);

        // Despite being nearer, the temp-zone shelf is skipped.
        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), Some((SHELF2.into(), 0)));
    }

    #[test]
    fn temp_zone_shelf_named_under_object_data_is_excluded_too() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        // Some frames keep the object payload nested under `object_data`.
        let mut frame = shelf_frame(SHELF1, (5.0, 0.0, 0.0), 0.0, all_free());
        frame.data["object_data"] = json!({"name": "shelf_cargo_temp_zone"});
        world.apply(&frame);
        let (ctx, _rx) = dummy_ctx(world);
        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), None);
    }

    #[test]
    fn unnamed_shelf_next_to_the_temp_zone_is_excluded_geometrically() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!({})));
        // No `name` replicated at all — but it stands right at the TempZone
        // place target (-5.354, 0.993, 9.46): it IS the temp rack.
        world.apply(&shelf_frame(SHELF1, (-5.4, 0.0, 9.5), 0.0, all_free()));
        world.apply(&shelf_frame(SHELF2, (20.0, 0.0, 0.0), 0.0, all_free()));
        let (ctx, _rx) = dummy_ctx(world);

        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), Some((SHELF2.into(), 0)));
    }

    #[test]
    fn shelf_parented_elsewhere_is_skipped() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({})));
        let mut frame = shelf_frame(SHELF1, (5.0, 0.0, 0.0), 0.0, all_free());
        frame.data["parent_id"] = json!("some-other-depot");
        world.apply(&frame);
        let (ctx, _rx) = dummy_ctx(world);
        assert_eq!(action().select_shelf(&ctx, &HashSet::new()), None);
    }

    #[test]
    fn cannot_run_when_not_at_work() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        let (ctx, _rx) = dummy_ctx(world);
        assert!(!action().can_run(&ctx));
    }

    #[test]
    fn runs_when_a_box_waits_in_the_temp_zone() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(action().can_run(&ctx));
    }

    #[test]
    fn empty_temp_zone_does_not_run() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({"zone_0": []})));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(!action().can_run(&ctx));
    }

    #[test]
    fn reception_owner_only_helps_stock_when_reception_is_idle() {
        use crate::depot::DepotRole;
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        let (mut ctx, _rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);

        // Reception owner, counter idle: it helps stock.
        ctx.depot_role = DepotRole::Reception;
        assert!(action().can_run(&ctx));

        // A box lands on the reception counter: reception is no longer idle, so
        // the owner stops stocking and tends the counter instead.
        world.apply(&GorcFrame {
            kind: GorcKind::UpdateProperty,
            channel: 0,
            object_id: DEPOT.into(),
            object_type: "cargo_depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "pnj_reception_deposited_item": ["box-9"] }),
        });
        assert!(!action().can_run(&ctx));

        // Counter cleared again -> back to helping.
        world.apply(&GorcFrame {
            kind: GorcKind::UpdateProperty,
            channel: 0,
            object_id: DEPOT.into(),
            object_type: "cargo_depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "pnj_reception_deposited_item": [] }),
        });
        assert!(action().can_run(&ctx));

        // But not while it still owes the walk back to its reception spot.
        ctx.state.set(
            crate::actions::depot_store_reception_to_temp_area::RETURN_HOME_KEY,
            json!(true),
        );
        assert!(!action().can_run(&ctx));
        ctx.state
            .remove(crate::actions::depot_store_reception_to_temp_area::RETURN_HOME_KEY);

        // A stocker stocks while the counter is empty.
        ctx.depot_role = DepotRole::Stocker;
        assert!(action().can_run(&ctx));
    }

    #[test]
    fn stocker_waits_out_a_fresh_deposit_instead_of_starting_a_carry() {
        use crate::depot::DepotRole;
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        let (mut ctx, _rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.depot_role = DepotRole::Stocker;
        assert!(action().can_run(&ctx));

        // A box lands on the counter. The stocker holds off: a shelf carry runs
        // far longer than the grace, so starting one now would leave the box
        // waiting even after the reception NPC has visibly failed to take it.
        world.apply(&GorcFrame {
            kind: GorcKind::UpdateProperty,
            channel: 0,
            object_id: DEPOT.into(),
            object_type: "cargo_depot".into(),
            player_id: None,
            timestamp: None,
            sub_event: None,
            data: json!({ "pnj_reception_deposited_item": ["box-9"] }),
        });
        assert!(!action().can_run(&ctx));

        // Once the grace passes the counter is the stocker's to clear, and
        // `depot_store_reception_to_temp_area` (priority 95) outranks stocking —
        // so this action is eligible again rather than deadlocked on a box
        // nobody accepts.
        {
            let mut depot = world.objects.get_mut(DEPOT).unwrap();
            let then = std::time::Instant::now() - Duration::from_secs(30);
            depot
                .props_changed_at
                .insert("pnj_reception_deposited_item".into(), then);
        }
        assert!(action().can_run(&ctx));
    }

    #[test]
    fn box_carried_by_a_player_is_not_takeable() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&player_frame("p1"));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        world.apply(&reparent_frame(ITEM, "p1"));
        let (ctx, _rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);

        let a = action();
        assert!(!a.can_run(&ctx));

        // Dropped back in the zone (reparented off the player): takeable now.
        world.apply(&reparent_frame(ITEM, ""));
        assert!(a.can_run(&ctx));
    }

    #[test]
    fn box_listed_but_not_in_view_is_not_takeable() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(!action().can_run(&ctx));
    }

    #[test]
    fn reception_carry_blocks_this_action() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        ctx.state.set(RECEPTION_BOX_IN_HANDS_KEY, json!("some-box"));
        assert!(!action().can_run(&ctx));
        ctx.state.remove(RECEPTION_BOX_IN_HANDS_KEY);
        assert!(action().can_run(&ctx));
    }

    #[test]
    fn own_carry_keeps_the_action_eligible() {
        let world = Arc::new(WorldView::new("me".into()));
        // Nothing in the temp zone anymore, but the box is in hands.
        world.apply(&depot_frame(json!({})));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(SHELF_BOX_IN_HANDS_KEY, json!(ITEM));
        assert!(action().can_run(&ctx));
    }

    #[test]
    fn head_aim_works_when_the_depot_ancestors_are_not_in_view() {
        let world = Arc::new(WorldView::new("me".into()));
        // The depot rides a planet that never replicated to this NPC: any
        // root-frame resolution fails…
        let mut depot = depot_frame(json!({}));
        depot.data["parent_id"] = json!("planet-never-seen");
        world.apply(&depot);
        // …but the NPC is parented to the depot, standing 1 m in front of
        // the slot at (9.5, 0.35, 5.4) — everything is depot-local.
        let mut me = self_frame("me", (9.5, 0.0, 6.4));
        me.data["parent_id"] = json!(DEPOT);
        world.apply(&me);
        let (ctx, _rx) = dummy_ctx(world);
        assert!(ctx.world.my_global_position().is_none(), "premise: globals must fail");

        let (pitch, eye_dist) =
            aim_at_local(&ctx, DEPOT, Vec3 { x: 9.5, y: 0.35, z: 5.4 })
                .expect("the head aim must not need the depot's ancestors");
        // Eyes at (9.5, 1.8, 6.4) looking at (9.5, 0.35, 5.4).
        let expected = (0.35f64 - 1.8).atan2(1.0);
        assert!((pitch - expected).abs() < 1e-9);
        assert!((eye_dist - 1.0f64.hypot(1.45)).abs() < 1e-9);
    }

    #[tokio::test]
    async fn no_usable_shelf_leaves_the_box_in_the_temp_zone() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        // Only a full shelf: the pre-flight check must refuse to grab.
        world.apply(&shelf_frame(SHELF1, (5.0, 0.0, 0.0), 0.0, all_full()));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        action().run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err(), "no walk order with nowhere to store");
        assert!(ctx.state.get(SHELF_BOX_IN_HANDS_KEY).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn full_happy_path_grabs_in_the_temp_zone_and_stores_on_the_shelf() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!({"zone_0": [ITEM]})));
        world.apply(&item_frame(ITEM, (-4.5, 0.5, 9.4)));
        world.apply(&shelf_frame(SHELF1, (10.0, 0.0, 5.0), 0.0, all_free()));
        let (mut ctx, mut rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);

        // Feed the two arrival notices and the two reparent confirmations
        // while the run is under way.
        let server = async {
            time::sleep(Duration::from_secs(2)).await;
            world.apply(&arrived_frame("me")); // at the temp place
            time::sleep(Duration::from_secs(5)).await;
            world.apply(&reparent_frame(ITEM, "me")); // grab confirmed
            time::sleep(Duration::from_secs(3)).await;
            world.apply(&arrived_frame("me")); // in front of the shelf
            time::sleep(Duration::from_secs(5)).await;
            world.apply(&reparent_frame(ITEM, "")); // drop confirmed
        };
        let a = action();
        let (result, ()) = tokio::join!(a.run(&mut ctx), server);
        result.unwrap();

        let msgs = drain(&mut rx);

        // 1. Walk order to a random point inside the TempZone rectangle.
        assert_eq!(msgs[0]["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(msgs[0]["data"]["uuid"], json!(DEPOT));
        let (x, y, z) = (
            msgs[0]["data"]["position"]["x"].as_f64().unwrap(),
            msgs[0]["data"]["position"]["y"].as_f64().unwrap(),
            msgs[0]["data"]["position"]["z"].as_f64().unwrap(),
        );
        assert!((x - TEMP0_CENTER.0).abs() <= TEMP_SIZE.0 / 2.0);
        assert_eq!(y, TEMP0_CENTER.1);
        assert!((z - TEMP0_CENTER.2).abs() <= TEMP_SIZE.2 / 2.0);

        // 2. Arrival acknowledged; 3. face the box; 4. head aim; 5. grab.
        assert_eq!(msgs[1]["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(msgs[2]["data"]["action"], json!("npc_face_position"));
        assert_eq!(msgs[2]["data"]["uuid"], json!(ITEM));
        assert_eq!(msgs[3]["data"]["action"], json!("update_property"));
        assert!(msgs[3]["data"]["head"].is_number());
        assert_eq!(msgs[4]["data"], json!({"action": "action", "target_uuid": ITEM}));

        // 6. Walk order to the computed standing point: slot 0's column,
        // 1 m in front of the shelf face, at floor height.
        assert_eq!(msgs[5]["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(msgs[5]["data"]["uuid"], json!(DEPOT));
        approx(msgs[5]["data"]["position"]["x"].as_f64().unwrap(), 9.5);
        approx(msgs[5]["data"]["position"]["y"].as_f64().unwrap(), 0.0);
        approx(msgs[5]["data"]["position"]["z"].as_f64().unwrap(), 6.4);

        // 7. Arrival acknowledged; 8. face the slot point (depot-local);
        // 9. head aim; 10. release.
        assert_eq!(msgs[6]["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(msgs[7]["data"]["action"], json!("npc_face_position"));
        assert_eq!(msgs[7]["data"]["uuid"], json!(DEPOT));
        approx(msgs[7]["data"]["position"]["x"].as_f64().unwrap(), 9.5);
        approx(msgs[7]["data"]["position"]["y"].as_f64().unwrap(), 0.35);
        approx(msgs[7]["data"]["position"]["z"].as_f64().unwrap(), 5.4);
        assert_eq!(msgs[8]["data"]["action"], json!("update_property"));
        assert!(msgs[8]["data"]["head"].is_number());
        assert_eq!(msgs[9]["data"], json!({"action": "action", "target_uuid": ITEM}));
        assert_eq!(msgs.len(), 10);

        assert!(ctx.state.get(SHELF_BOX_IN_HANDS_KEY).is_none(), "carry consumed");
    }

    #[tokio::test(start_paused = true)]
    async fn slot_filled_during_the_walk_repicks_another_shelf() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!({})));
        world.apply(&item_frame(ITEM, (0.3, 1.0, 0.3)));
        world.apply(&reparent_frame(ITEM, "me"));
        world.apply(&shelf_frame(SHELF1, (5.0, 0.0, 5.0), 0.0, all_free()));
        world.apply(&shelf_frame(SHELF2, (30.0, 0.0, 5.0), 0.0, all_free()));
        let (mut ctx, mut rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(SHELF_BOX_IN_HANDS_KEY, json!(ITEM));

        // While the NPC walks to the near shelf, every one of its slots gets
        // filled; the drop must re-pick and land on the far shelf.
        let server = async {
            time::sleep(Duration::from_secs(2)).await;
            world.apply(&shelf_frame(SHELF1, (5.0, 0.0, 5.0), 0.0, all_full()));
            world.apply(&arrived_frame("me"));
            time::sleep(Duration::from_secs(3)).await;
            world.apply(&arrived_frame("me")); // in front of the far shelf
            time::sleep(Duration::from_secs(5)).await;
            world.apply(&reparent_frame(ITEM, "")); // drop confirmed
        };
        let a = action();
        let (result, ()) = tokio::join!(a.run(&mut ctx), server);
        result.unwrap();

        let msgs = drain(&mut rx);
        // Walk to shelf 1 (stand x = 5 - 0.5), ack — then NO drop there:
        // straight to shelf 2 (stand x = 30 - 0.5), ack, face, aim, release.
        assert_eq!(msgs[0]["data"]["action"], json!("npc_go_to_position"));
        approx(msgs[0]["data"]["position"]["x"].as_f64().unwrap(), 4.5);
        assert_eq!(msgs[1]["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(msgs[2]["data"]["action"], json!("npc_go_to_position"));
        approx(msgs[2]["data"]["position"]["x"].as_f64().unwrap(), 29.5);
        assert_eq!(msgs[3]["data"]["action"], json!("npc_arrived_ack"));
        assert_eq!(msgs[4]["data"]["action"], json!("npc_face_position"));
        approx(msgs[4]["data"]["position"]["x"].as_f64().unwrap(), 29.5);
        assert_eq!(msgs[5]["data"]["action"], json!("update_property"));
        assert_eq!(msgs[6]["data"], json!({"action": "action", "target_uuid": ITEM}));
        assert_eq!(msgs.len(), 7);

        assert!(ctx.state.get(SHELF_BOX_IN_HANDS_KEY).is_none(), "carry consumed");
    }

    #[tokio::test]
    async fn every_shelf_full_mid_carry_keeps_the_box_for_later() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!({})));
        world.apply(&item_frame(ITEM, (0.3, 1.0, 0.3)));
        world.apply(&reparent_frame(ITEM, "me"));
        world.apply(&shelf_frame(SHELF1, (5.0, 0.0, 5.0), 0.0, all_full()));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(SHELF_BOX_IN_HANDS_KEY, json!(ITEM));

        let a = action();
        a.run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err(), "no walk order while every shelf is full");
        assert_eq!(ctx.state.get(SHELF_BOX_IN_HANDS_KEY), Some(json!(ITEM)));
        assert!(a.can_run(&ctx), "the carry must retry once a slot frees up");
    }

    #[tokio::test]
    async fn lost_box_abandons_the_carry() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!({})));
        // The box is back on the ground (server auto-drop): not in hands.
        world.apply(&item_frame(ITEM, (0.3, 0.5, 0.3)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(SHELF_BOX_IN_HANDS_KEY, json!(ITEM));

        action().run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err(), "nothing to carry, nothing sent");
        assert!(ctx.state.get(SHELF_BOX_IN_HANDS_KEY).is_none());
    }
}
