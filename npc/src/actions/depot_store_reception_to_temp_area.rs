use std::time::Duration;

use async_trait::async_trait;
use rand::Rng;
use serde_json::{Value, json};
use tokio::time;
use tracing::{info, warn};

use super::{Action, ActionContext, ActionTiming};
use crate::{
    error::Result,
    horizon::messages::Vec3,
    npc::{spec::DepotStoreReceptionToTempAreaConfig, state::NpcStatus},
};

/// How often the NPC checks the reception counter while at work.
const CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Depot property carrying what is currently deposited on the reception
/// counter: an array of item UUIDs (a bare string is treated as one item).
const DEPOSITED_PROP: &str = "pnj_reception_deposited_item";

/// Depot property listing the players waiting at the reception; the NPC's
/// replies are addressed to them.
const RECEPTION_PLAYERS_PROP: &str = "reception_players";

/// Depot property mapping `zone_<n>` to the box UUIDs currently sitting in
/// the matching `TempZone/PNJPlace<n>` spot. Kept by the depot itself (its
/// deposit areas watch bodies entering/leaving); a missing key or an empty
/// array both mean the spot is free.
const BOX_TEMP_ZONES_PROP: &str = "pnj_box_temp_zones";

/// Depot property describing the NPC work spots:
/// `{zone: {place: {path, position: [x,y,z], size: [x,y,z]}}}`, positions
/// local to the depot.
const PNJ_ZONES_PROP: &str = "pnj_zones";

/// Zone / place (within [`PNJ_ZONES_PROP`]) the NPC stands in to pick a box up
/// from the reception counter.
const RECEPTION_ZONE: &str = "Reception";
const RECEPTION_PLACE: &str = "PNJPlace0";

/// Zone (within [`PNJ_ZONES_PROP`]) listing the temp storage places the NPC
/// carries accepted boxes to. Each place additionally carries a `target`
/// point — where on the shelf the box goes — to aim the head at before the
/// drop.
const TEMP_ZONE: &str = "TempZone";

/// Blackboard key remembering the last [`DEPOSITED_PROP`] value this action
/// handled, so each deposit is checked exactly once.
const LAST_DEPOSITED_KEY: &str = "depot_store_reception_last_deposited";

/// Blackboard key holding the UUID of an accepted box between validation and
/// the end of its pickup.
pub const ACCEPTED_ITEM_KEY: &str = "depot_store_reception_accepted_item";

/// Blackboard key holding the UUID of the box the NPC picked up. Set by the
/// pickup step and consumed by the carry-to-temp-area step once the drop is
/// confirmed.
pub const BOX_IN_HANDS_KEY: &str = "depot_store_reception_box_in_hands";

/// What the NPC says when more than one item sits on the counter.
const ONLY_ONE_OBJECT: &str = "Please, put only 1 object";

/// What the NPC says when every temp-zone place already holds a box, so a
/// deposit cannot be accepted right now — followed by [`NEED_TO_STORE`].
const NO_MORE_PLACE: &str = "Sorry, no more place";
const NEED_TO_STORE: &str = "I need to put boxes away in the warehouse";

/// Eye height above the NPC's feet, matching the player rig; the head pitch is
/// computed from here toward the box centre.
const HEAD_HEIGHT: f64 = 1.80;

/// How long to wait for the server's `npc_arrived` notice before abandoning
/// the trip (and re-arming the deposit check for a retry).
const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// Poll cadence while waiting for `npc_arrived`.
const ARRIVAL_POLL: Duration = Duration::from_millis(200);

/// Fallback arrival gate: if the NPC's replicated position gets this close
/// (ground plane, metres) to the walk target, proceed even when the server
/// never sends `npc_arrived` (its own gate needs `<= 1.5 m` AND an empty
/// path, which a partial route can miss).
const ARRIVAL_FALLBACK_DIST: f64 = 0.7;

/// Pause between arriving at the reception place and grabbing the box.
const SETTLE_BEFORE_PICKUP: Duration = Duration::from_secs(2);

/// How long to let the server finish an `npc_face_position` pivot before
/// aiming the head: a worst-case half turn at the server's half-turn-per-
/// second speed, plus replication margin.
const TURN_WAIT: Duration = Duration::from_millis(1300);

/// Pause between facing/aiming and pressing the action key, so the rotation
/// replicates and the gesture reads as deliberate.
const PAUSE_BEFORE_ACTION: Duration = Duration::from_millis(500);

/// How long to wait for the server to confirm the grab — a successful pickup
/// reparents the box under the NPC, which replicates back as the box's
/// `parent_id`.
const GRAB_CONFIRM_TIMEOUT: Duration = Duration::from_secs(4);

/// Poll cadence while waiting for the grab confirmation.
const GRAB_CONFIRM_POLL: Duration = Duration::from_millis(200);

/// Pause between arriving at the temp place and dropping the box.
const SETTLE_BEFORE_DROP: Duration = Duration::from_secs(2);

/// How long to wait for the server to confirm the drop — releasing the box
/// reparents it off the NPC, which replicates back as the box's `parent_id`.
const DROP_CONFIRM_TIMEOUT: Duration = Duration::from_secs(4);

/// While the NPC is [`AtWork`](NpcStatus::AtWork), watches the depot it works
/// at — the object whose UUID is the NPC's configured `place_where_work`.
/// Whenever the depot's `pnj_reception_deposited_item` property changes, the
/// deposit is validated: exactly one item, whose `scenename` is listed in the
/// per-NPC `boxes_type_allowed` config, and at least one free temp-zone place
/// to put it (checked BEFORE the pickup — a box grabbed with the temp area
/// full could never be put down). Rule violations are told to the players
/// waiting at the reception.
///
/// An accepted box is then picked up: the NPC walks to a random point inside
/// the depot's `pnj_zones.Reception.PNJPlace0` rectangle (like the trip to
/// work: `npc_go_to_position` answered by `npc_arrived`), settles for a
/// second, turns to face the box (an `npc_face_position` order: the server
/// pivots the body around its own up axis at half a turn per second, like a
/// human), aims its head at the box centre, pauses, and presses the action
/// key on it. The box UUID then moves to [`BOX_IN_HANDS_KEY`] on the
/// blackboard.
///
/// With a box in hands, the next run carries it to the temp area: a free
/// `pnj_zones.TempZone` place is chosen (its `zone_<n>` entry in the depot's
/// `pnj_box_temp_zones` is missing or empty — the depot maintains that map
/// itself as boxes enter/leave its deposit areas), the NPC walks to a random
/// point inside its rectangle, turns to face the place's `target` shelf
/// point (same `npc_face_position` pivot), aims its head at it, pauses, and
/// presses the action key again — which releases a carried item.
/// The drop only counts once the box replicates as reparented off the NPC;
/// otherwise the carry retries on a later run.
pub struct DepotStoreReceptionToTempArea {
    /// UUID of the depot the NPC works at, taken from `place_where_work`.
    /// `None` means the NPC has no workplace, so this action never runs.
    work_uuid: Option<String>,
    /// Accepted box scene file names (e.g. `hauling_box.tscn`). Empty (no
    /// config) disables the action rather than rejecting everything.
    boxes_type_allowed: Vec<String>,
}

impl DepotStoreReceptionToTempArea {
    pub fn new(
        work_uuid: Option<String>,
        config: Option<DepotStoreReceptionToTempAreaConfig>,
    ) -> Self {
        Self {
            work_uuid,
            boxes_type_allowed: config.map(|c| c.boxes_type_allowed).unwrap_or_default(),
        }
    }

    /// Current `pnj_reception_deposited_item` value, but only when it differs
    /// from the last one this action handled. `None` when the NPC has no
    /// workplace, the depot is not in view, the property is absent, or nothing
    /// changed since the last check.
    fn pending_deposit(&self, ctx: &ActionContext) -> Option<Value> {
        let uuid = self.work_uuid.as_ref()?;
        let depot = ctx.world.objects.get(uuid)?;
        let current = depot.property(DEPOSITED_PROP)?.clone();
        if ctx.state.get(LAST_DEPOSITED_KEY).as_ref() == Some(&current) {
            return None;
        }
        Some(current)
    }

    /// Item UUIDs held in a `pnj_reception_deposited_item` value. Accepts an
    /// array of strings or a bare string; anything else counts as no items.
    fn deposited_items(value: &Value) -> Vec<String> {
        match value {
            Value::Array(a) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Value::String(s) if !s.is_empty() => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    /// Whether an item's scene is one of the accepted box types. The
    /// replicated `scenename` may be a full `res://…` path, so the comparison
    /// is on the file name.
    fn is_allowed(&self, scenename: &str) -> bool {
        let base = scenename.rsplit('/').next().unwrap_or(scenename);
        self.boxes_type_allowed.iter().any(|a| a == base)
    }

    /// Accepted box types as shown to players: `.tscn` stripped, joined by
    /// commas.
    fn allowed_label(&self) -> String {
        self.boxes_type_allowed
            .iter()
            .map(|s| s.strip_suffix(".tscn").unwrap_or(s))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Random standing point inside the depot's `Reception/PNJPlace0`
    /// rectangle, in the depot's local frame: x and z uniform within the
    /// rectangle, y at the rectangle's own height. `None` when the depot or
    /// the zone description is not in view.
    fn reception_place_point(&self, ctx: &mut ActionContext) -> Option<Vec3> {
        let uuid = self.work_uuid.as_ref()?;
        let (center, size) = {
            let depot = ctx.world.objects.get(uuid)?;
            let place = depot
                .property(PNJ_ZONES_PROP)?
                .get(RECEPTION_ZONE)?
                .get(RECEPTION_PLACE)?
                .clone();
            (vec3_from(place.get("position")?)?, vec3_from(place.get("size")?)?)
        };
        Some(Vec3 {
            x: center.x + ctx.rng.gen_range(-size.x / 2.0..=size.x / 2.0),
            y: center.y,
            z: center.z + ctx.rng.gen_range(-size.z / 2.0..=size.z / 2.0),
        })
    }

    /// The depot's free `pnj_zones.TempZone` places — those whose `zone_<n>`
    /// entry in `pnj_box_temp_zones` holds no box — as
    /// `(name, position, size, target)`. `None` when the depot or the zone
    /// description is not in view; `Some` with an empty vec when every place
    /// is occupied (a different situation: the counter must refuse deposits).
    fn free_temp_places(&self, ctx: &ActionContext) -> Option<Vec<(String, Vec3, Vec3, Vec3)>> {
        let uuid = self.work_uuid.as_ref()?;
        let (places, occupied) = {
            let depot = ctx.world.objects.get(uuid)?;
            (
                depot.property(PNJ_ZONES_PROP)?.get(TEMP_ZONE)?.as_object()?.clone(),
                depot
                    .property(BOX_TEMP_ZONES_PROP)
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        };
        let vacant: Vec<(&String, &Value)> = places
            .iter()
            .filter(|(name, _)| {
                zone_key(name).is_some_and(|k| {
                    !occupied
                        .get(&k)
                        .and_then(Value::as_array)
                        .is_some_and(|boxes| !boxes.is_empty())
                })
            })
            .collect();
        let parsed: Vec<(String, Vec3, Vec3, Vec3)> = vacant
            .iter()
            .filter_map(|(name, place)| {
                Some((
                    (*name).clone(),
                    vec3_from(place.get("position")?)?,
                    vec3_from(place.get("size")?)?,
                    vec3_from(place.get("target")?)?,
                ))
            })
            .collect();
        // A vacant place that fails to parse must read as "zone data broken,
        // retry later" — NEVER as "temp area full": refusing deposits over a
        // data hiccup would lie to the players. Logging the raw entry tells
        // us which encoding the depot actually replicated.
        if parsed.is_empty() && !vacant.is_empty() {
            warn!(
                npc = %ctx.npc_name,
                place = %vacant[0].0,
                raw = %vacant[0].1,
                "vacant temp-zone places present but unparseable; treating as not in view"
            );
            return None;
        }
        if vacant.is_empty() {
            info!(
                npc = %ctx.npc_name,
                occupied = %occupied,
                places = places.len(),
                "every temp-zone place is occupied"
            );
        }
        Some(parsed)
    }

    /// A free temp-zone place to store the carried box at, picked at random
    /// among [`free_temp_places`](Self::free_temp_places). Returns the place
    /// name, a random standing point inside its rectangle (depot-local, like
    /// the reception spot), and the `target` shelf point to aim at. `None`
    /// when the zone description is not in view or every place is occupied.
    fn free_temp_place(&self, ctx: &mut ActionContext) -> Option<(String, Vec3, Vec3)> {
        let free = self.free_temp_places(ctx)?;
        if free.is_empty() {
            return None;
        }
        let (name, center, size, target) = free[ctx.rng.gen_range(0..free.len())].clone();
        Some((
            name,
            Vec3 {
                x: center.x + ctx.rng.gen_range(-size.x / 2.0..=size.x / 2.0),
                y: center.y,
                z: center.z + ctx.rng.gen_range(-size.z / 2.0..=size.z / 2.0),
            },
            target,
        ))
    }

    /// Tell every player waiting at the reception `text`.
    async fn say_to_reception(&self, ctx: &mut ActionContext, text: &str) -> Result<()> {
        // Collect before awaiting so no map guard is held across the sends.
        let players: Vec<String> = match self.work_uuid.as_ref().and_then(|u| ctx.world.objects.get(u)) {
            Some(depot) => depot
                .property(RECEPTION_PLAYERS_PROP)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if players.is_empty() {
            warn!(npc = %ctx.npc_name, %text, "no player at the reception to address");
            return Ok(());
        }
        for player in players {
            info!(npc = %ctx.npc_name, player = %player, %text, "reception deposit feedback");
            ctx.send
                .send(
                    "player",
                    "client_action",
                    json!({
                        "action": "dialog",
                        "to": player,
                        "text": text,
                        "uuid": ctx.world.my_uuid,
                    }),
                )
                .await?;
        }
        Ok(())
    }

    /// Walk to `target` (local to the depot), wait for the server's arrival
    /// notice (or reach the spot per our own position), settle, aim the head
    /// at the box, and press the action key on it. The session loop keeps
    /// feeding the world view while this awaits, so `npc_arrived` is observed
    /// mid-run. The pickup only counts once the server reparents the box
    /// under the NPC; otherwise the deposit check re-arms and the whole flow
    /// retries.
    async fn pick_up(&self, ctx: &mut ActionContext, item: &str, target: Vec3) -> Result<()> {
        let Some(depot) = self.work_uuid.clone() else {
            return Ok(());
        };
        // A stale arrival flag from a previous trip must not fake this one.
        ctx.world.clear_arrived();
        info!(npc = %ctx.npc_name, item = %item, "walking to the reception place");
        ctx.send
            .send(
                "player",
                "client_action",
                json!({
                    "action": "npc_go_to_position",
                    "uuid": depot,
                    "position": target,
                }),
            )
            .await?;

        match wait_for_arrival(ctx, &depot, target, ARRIVAL_TIMEOUT).await {
            ArrivalOutcome::Notified => {
                info!(npc = %ctx.npc_name, item = %item, "arrived at the reception place");
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
                info!(npc = %ctx.npc_name, item = %item, "close enough to the reception place (no server arrival notice)");
            }
            ArrivalOutcome::TimedOut => {
                // Neither signal is trustworthy on its own (replicated frames
                // have disagreed in practice): log the gap, then attempt the
                // grab anyway — the outcome is confirmed below either way.
                let dist = ground_distance_to(ctx, &depot, target)
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

        // The player may have taken the box back during the walk/settle: a
        // carried box cannot be grabbed, so re-arm and retry instead of
        // pressing the action key on someone's hands.
        if carried_by_player(ctx, item) {
            warn!(npc = %ctx.npc_name, item = %item, "box back in a player's hands; abandoning pickup for a retry");
            ctx.state.remove(LAST_DEPOSITED_KEY);
            ctx.state.remove(ACCEPTED_ITEM_KEY);
            return Ok(());
        }

        // Face the box first, so the grab does not happen over the shoulder.
        // The server pivots the body smoothly (half a turn per second) around
        // its own up axis — upright in any gravity frame, which yaw angles
        // sent from here could not guarantee. The box is the frame reference,
        // so its own origin is the point to face.
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

        // Aim the head at the box centre so the server-side gaze (interaction
        // ray, carried-item hold point) matches what the NPC is grabbing.
        match aim_at(ctx, item) {
            Some((pitch, eye_dist)) => {
                info!(
                    npc = %ctx.npc_name,
                    item = %item,
                    pitch = format!("{pitch:.3}"),
                    eye_dist_m = format!("{eye_dist:.2}"),
                    "aiming head at the box"
                );
                // The server rejects/auto-drops a carry farther than the
                // interact reach (3 m eye-to-box), and nav arrival forgives up
                // to 1.5 m — surface a too-far stand point instead of failing
                // silently.
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

        // A successful grab reparents the box under the NPC and replicates
        // that back — only then is the pickup real. Anything else (too far,
        // no line of sight, box gone) re-arms the whole flow for a retry.
        if wait_for_box_in_hands(ctx, item, GRAB_CONFIRM_TIMEOUT).await {
            info!(npc = %ctx.npc_name, item = %item, "box in hands");
            ctx.state.remove(ACCEPTED_ITEM_KEY);
            ctx.state.set(BOX_IN_HANDS_KEY, json!(item));
        } else {
            warn!(npc = %ctx.npc_name, item = %item, "grab not confirmed by the server; abandoning for a retry");
            ctx.state.remove(LAST_DEPOSITED_KEY);
            ctx.state.remove(ACCEPTED_ITEM_KEY);
        }
        Ok(())
    }

    /// Carry the box in hands to a free temp-zone place and drop it there:
    /// walk to a random point inside the place's rectangle, wait for arrival
    /// (same protocol as the pickup trip), settle, aim the head at the place's
    /// `target` shelf point, and press the action key — which releases a
    /// carried item. The drop only counts once the box replicates as
    /// reparented off the NPC; otherwise [`BOX_IN_HANDS_KEY`] stays set and
    /// the carry retries on a later run. The depot updates its own
    /// `pnj_box_temp_zones` when the box lands in the deposit area.
    async fn store_in_temp_zone(&self, ctx: &mut ActionContext, item: &str) -> Result<()> {
        let Some(depot) = self.work_uuid.clone() else {
            return Ok(());
        };
        // The carry may have been lost on the way (e.g. the server auto-drops
        // an out-of-reach item): with nothing in hands the action key would
        // GRAB, not release, so re-arm the whole reception flow instead.
        let held = ctx
            .world
            .objects
            .get(item)
            .and_then(|o| o.parent_id.clone())
            .is_some_and(|p| p == ctx.world.my_uuid);
        if !held {
            warn!(npc = %ctx.npc_name, item = %item, "box no longer in hands; abandoning the carry");
            ctx.state.remove(BOX_IN_HANDS_KEY);
            ctx.state.remove(LAST_DEPOSITED_KEY);
            return Ok(());
        }

        // Every place occupied (or zone data not replicated yet): keep the
        // box and retry on a later run.
        let Some((place, stand, shelf)) = self.free_temp_place(ctx) else {
            warn!(npc = %ctx.npc_name, item = %item, "no free temp-zone place; retrying later");
            return Ok(());
        };

        ctx.world.clear_arrived();
        info!(npc = %ctx.npc_name, item = %item, place = %place, "carrying the box to the temp area");
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
                info!(npc = %ctx.npc_name, item = %item, place = %place, "arrived at the temp-zone place");
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
                info!(npc = %ctx.npc_name, item = %item, place = %place, "close enough to the temp-zone place (no server arrival notice)");
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

        // Face the shelf first, so the box is released toward the shelf and
        // not over the shoulder. The server pivots the body smoothly (half a
        // turn per second) around its own up axis; the shelf point is local
        // to the depot — exactly the {uuid, position} contract of the action.
        info!(npc = %ctx.npc_name, item = %item, place = %place, "turning to face the shelf");
        ctx.send
            .send(
                "player",
                "client_action",
                json!({
                    "action": "npc_face_position",
                    "uuid": depot,
                    "position": shelf,
                }),
            )
            .await?;
        time::sleep(TURN_WAIT).await;

        // Aim the head at the shelf point so the carried box — which follows
        // the gaze — hangs toward the shelf when released.
        match aim_at_local(ctx, &depot, shelf) {
            Some((pitch, eye_dist)) => {
                info!(
                    npc = %ctx.npc_name,
                    item = %item,
                    pitch = format!("{pitch:.3}"),
                    eye_dist_m = format!("{eye_dist:.2}"),
                    "aiming head at the shelf point"
                );
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

        info!(npc = %ctx.npc_name, item = %item, place = %place, "dropping the box");
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
            info!(npc = %ctx.npc_name, item = %item, place = %place, "box stored in the temp area");
            ctx.state.remove(BOX_IN_HANDS_KEY);
        } else {
            warn!(npc = %ctx.npc_name, item = %item, "drop not confirmed by the server; retrying later");
        }
        Ok(())
    }
}

/// `pnj_box_temp_zones` key for a `TempZone` place: `PNJPlace4` → `zone_4`
/// (the digits ending the place name). `None` for a name without a trailing
/// number.
fn zone_key(place: &str) -> Option<String> {
    let digits = place.trim_start_matches(|c: char| !c.is_ascii_digit());
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("zone_{digits}"))
}

/// UUID of the box currently in the NPC's hands, per the blackboard.
fn box_in_hands(ctx: &ActionContext) -> Option<String> {
    ctx.state
        .get(BOX_IN_HANDS_KEY)
        .and_then(|v| v.as_str().map(str::to_string))
}

/// Whether `item` currently rides in someone's hands: the server reparents a
/// carried prop under its carrier, so a player uuid (the NPC's own included)
/// as `parent_id` means the box is not restable-on-the-counter yet.
fn carried_by_player(ctx: &ActionContext, item: &str) -> bool {
    let Some(parent) = ctx.world.objects.get(item).and_then(|o| o.parent_id.clone()) else {
        return false;
    };
    parent == ctx.world.my_uuid || ctx.world.players.contains_key(&parent)
}

/// Head pitch (radians, Godot camera convention: negative looks down) that
/// points the NPC's eyes — [`HEAD_HEIGHT`] above its feet — at the centre of
/// `item`, plus the eye-to-item distance in metres (the server's grab reach is
/// 3 m). `None` while either global position cannot be resolved.
fn aim_at(ctx: &ActionContext, item: &str) -> Option<(f64, f64)> {
    let target = item_global(ctx, item)?;
    pitch_toward(ctx, target)
}

/// Global position of `item`, resolved through its parent chain. `None` while
/// the item or an ancestor is not in view.
fn item_global(ctx: &ActionContext, item: &str) -> Option<Vec3> {
    let (pos, parent) = {
        let o = ctx.world.objects.get(item)?;
        (o.position, o.parent_id.clone())
    };
    ctx.world.resolve_global(pos, parent.as_deref())
}

/// Same as [`aim_at`], but toward `point` expressed in `parent`'s local frame
/// (e.g. a temp-zone `target` local to the depot).
fn aim_at_local(ctx: &ActionContext, parent: &str, point: Vec3) -> Option<(f64, f64)> {
    let target = ctx.world.resolve_global(point, Some(parent))?;
    pitch_toward(ctx, target)
}

/// Head pitch and eye distance from the NPC's eyes to the global `target`.
fn pitch_toward(ctx: &ActionContext, target: Vec3) -> Option<(f64, f64)> {
    let npc = ctx.world.my_global_position()?;
    let dy = target.y - (npc.y + HEAD_HEIGHT);
    let (dx, dz) = (target.x - npc.x, target.z - npc.z);
    let horiz = dx.hypot(dz);
    Some((dy.atan2(horiz), horiz.hypot(dy)))
}

/// How a wait for the walk target ended.
enum ArrivalOutcome {
    /// The server flagged `npc_arrived` (needs an ack).
    Notified,
    /// No server notice, but the NPC's own position reached the target
    /// (within [`ARRIVAL_FALLBACK_DIST`]).
    CloseEnough,
    TimedOut,
}

/// Ground-plane distance (metres) from the NPC to `target` (local to `depot`),
/// or `None` while either global position cannot be resolved.
fn ground_distance_to(ctx: &ActionContext, depot: &str, target: Vec3) -> Option<f64> {
    let npc = ctx.world.my_global_position()?;
    let goal = ctx.world.resolve_global(target, Some(depot))?;
    Some((goal.x - npc.x).hypot(goal.z - npc.z))
}

/// Wait until the box replicates as parented under this NPC (grab confirmed),
/// or `timeout` elapses.
async fn wait_for_box_in_hands(ctx: &ActionContext, item: &str, timeout: Duration) -> bool {
    let deadline = time::Instant::now() + timeout;
    while time::Instant::now() < deadline {
        let in_hands = ctx
            .world
            .objects
            .get(item)
            .and_then(|o| o.parent_id.clone())
            .is_some_and(|p| p == ctx.world.my_uuid);
        if in_hands {
            return true;
        }
        time::sleep(GRAB_CONFIRM_POLL).await;
    }
    false
}

/// Wait until the box replicates as no longer parented under this NPC (drop
/// confirmed), or `timeout` elapses. A box gone from view also counts as
/// released.
async fn wait_for_box_released(ctx: &ActionContext, item: &str, timeout: Duration) -> bool {
    let deadline = time::Instant::now() + timeout;
    while time::Instant::now() < deadline {
        let held = ctx
            .world
            .objects
            .get(item)
            .and_then(|o| o.parent_id.clone())
            .is_some_and(|p| p == ctx.world.my_uuid);
        if !held {
            return true;
        }
        time::sleep(GRAB_CONFIRM_POLL).await;
    }
    false
}

/// Wait until the server flags `npc_arrived`, the NPC's replicated position
/// reaches `target`, or `timeout` elapses.
async fn wait_for_arrival(
    ctx: &ActionContext,
    depot: &str,
    target: Vec3,
    timeout: Duration,
) -> ArrivalOutcome {
    let deadline = time::Instant::now() + timeout;
    while time::Instant::now() < deadline {
        if ctx.world.npc_arrived() {
            return ArrivalOutcome::Notified;
        }
        if ground_distance_to(ctx, depot, target).is_some_and(|d| d <= ARRIVAL_FALLBACK_DIST) {
            return ArrivalOutcome::CloseEnough;
        }
        time::sleep(ARRIVAL_POLL).await;
    }
    ArrivalOutcome::TimedOut
}

/// A replicated vector to a [`Vec3`]. The pipeline is not consistent about
/// the encoding, so all three observed forms are accepted: a `[x, y, z]`
/// array (DB-exported `pnj_zones`), a `{x, y, z}` map (top-level frame
/// positions), and Godot's `"(x, y, z)"` string (a raw `Vector3` passed
/// through `JSON.stringify`).
fn vec3_from(v: &Value) -> Option<Vec3> {
    if let Some(a) = v.as_array() {
        return Some(Vec3 {
            x: a.first()?.as_f64()?,
            y: a.get(1)?.as_f64()?,
            z: a.get(2)?.as_f64()?,
        });
    }
    if v.is_object() {
        return Some(Vec3 {
            x: v.get("x")?.as_f64()?,
            y: v.get("y")?.as_f64()?,
            z: v.get("z")?.as_f64()?,
        });
    }
    let s = v.as_str()?.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut parts = s.split(',').map(|p| p.trim().parse::<f64>());
    Some(Vec3 {
        x: parts.next()?.ok()?,
        y: parts.next()?.ok()?,
        z: parts.next()?.ok()?,
    })
}

#[async_trait]
impl Action for DepotStoreReceptionToTempArea {
    fn name(&self) -> &'static str {
        "depot_store_reception_to_temp_area"
    }

    fn priority(&self) -> i32 {
        95
    }

    fn timing(&self) -> ActionTiming {
        // Poll the counter at ~0.5 Hz rather than on every scheduler tick.
        ActionTiming {
            min_interval: Some(CHECK_INTERVAL),
            ..Default::default()
        }
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        if ctx.state.status() != NpcStatus::AtWork {
            return false;
        }
        // A box already in hands must reach the temp area regardless of what
        // sits on the counter.
        if box_in_hands(ctx).is_some() {
            return true;
        }
        // Otherwise only with box types configured, and only when the deposit
        // changed since the last check.
        !self.boxes_type_allowed.is_empty() && self.pending_deposit(ctx).is_some()
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        // Finish carrying a picked-up box before looking at the counter again.
        if let Some(item) = box_in_hands(ctx) {
            return self.store_in_temp_zone(ctx, &item).await;
        }
        let Some(current) = self.pending_deposit(ctx) else {
            return Ok(());
        };
        let items = Self::deposited_items(&current);
        match items.as_slice() {
            [] => {
                // Counter cleared: nothing to validate, just remember it.
            }
            [item] => {
                let scenename = ctx
                    .world
                    .objects
                    .get(item)
                    .and_then(|o| o.scenename.clone());
                let Some(scenename) = scenename else {
                    // Item not replicated into view yet: leave the change
                    // unconsumed so the next tick retries the validation.
                    return Ok(());
                };
                if self.is_allowed(&scenename) {
                    // The counter area detects the box as soon as its body
                    // crosses it — usually while still in the depositing
                    // player's hands — and a carried box cannot be grabbed
                    // (`interact()` refuses it). Leave the change pending so
                    // this retries every tick until the player drops it.
                    if carried_by_player(ctx, item) {
                        info!(npc = %ctx.npc_name, item = %item, "box still held by a player; waiting for the drop");
                        return Ok(());
                    }
                    // Zone data can lag behind the depot's first frame; retry
                    // rather than accepting a box the NPC cannot walk to.
                    let Some(target) = self.reception_place_point(ctx) else {
                        warn!(npc = %ctx.npc_name, "pnj_zones Reception/PNJPlace0 not in view; retrying");
                        return Ok(());
                    };
                    // Check the temp area BEFORE picking the box up: grabbed
                    // with nowhere to put it down, it would stay in the NPC's
                    // hands forever.
                    match self.free_temp_places(ctx) {
                        None => {
                            warn!(npc = %ctx.npc_name, "pnj_zones TempZone not in view; retrying");
                            return Ok(());
                        }
                        Some(free) if free.is_empty() => {
                            info!(npc = %ctx.npc_name, item = %item, "temp area full; refusing the deposit");
                            self.say_to_reception(ctx, NO_MORE_PLACE).await?;
                            self.say_to_reception(ctx, NEED_TO_STORE).await?;
                            // Consume the change: one refusal per deposit, a
                            // new deposit (or this one re-dropped) re-checks.
                            ctx.state.set(LAST_DEPOSITED_KEY, current);
                            return Ok(());
                        }
                        Some(_) => {}
                    }
                    info!(npc = %ctx.npc_name, item = %item, %scenename, "deposited box accepted");
                    // Consume the change before the (long) pickup trip.
                    ctx.state.set(LAST_DEPOSITED_KEY, current);
                    ctx.state.set(ACCEPTED_ITEM_KEY, json!(item));
                    self.pick_up(ctx, item, target).await?;
                    return Ok(());
                }
                let text = format!("We only accept the type of box: {}", self.allowed_label());
                self.say_to_reception(ctx, &text).await?;
            }
            _ => {
                self.say_to_reception(ctx, ONLY_ONE_OBJECT).await?;
            }
        }
        ctx.state.set(LAST_DEPOSITED_KEY, current);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::{SeedableRng, rngs::StdRng};
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        horizon::client::HorizonSender,
        horizon::messages::{GorcFrame, GorcKind},
        npc::state::NpcState,
        world::view::WorldView,
    };

    const DEPOT: &str = "a9340169-767c-4b02-a2f3-67f2786375b3";
    const ITEM: &str = "11111111-1111-1111-1111-111111111111";

    // The Reception/PNJPlace0 rectangle used by `depot_frame`.
    const PLACE_CENTER: (f64, f64, f64) = (0.7165, 0.2509, 3.9028);
    const PLACE_SIZE: (f64, f64, f64) = (0.71996, 0.49816, 0.55629);

    // The two TempZone places used by `depot_frame`.
    const TEMP_SIZE: (f64, f64, f64) = (0.37, 0.5, 0.24);
    const TEMP0_CENTER: (f64, f64, f64) = (-4.5, 0.251, 9.471);
    const TEMP0_TARGET: (f64, f64, f64) = (-5.354, 0.993, 9.46);
    const TEMP1_CENTER: (f64, f64, f64) = (-4.5, 0.251, 8.982);
    const TEMP1_TARGET: (f64, f64, f64) = (-5.354, 0.993, 8.971);

    fn dummy_ctx(world: Arc<WorldView>) -> (ActionContext, tokio::sync::mpsc::Receiver<Value>) {
        // Wide enough for a full pickup/drop exchange — a full channel would
        // deadlock a paused-time test (send blocks, nothing drains).
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let ctx = ActionContext {
            npc_name: "Test".into(),
            world,
            state: Arc::new(NpcState::new()),
            send: HorizonSender::__test_new(tx),
            rng: StdRng::seed_from_u64(0),
        };
        (ctx, rx)
    }

    /// The NPC's depot entering a zone with a deposit, one waiting player, and
    /// the Reception/PNJPlace0 work spot.
    fn depot_frame(deposited: Value) -> GorcFrame {
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
                "reception_players": ["p1"],
                "pnj_reception_deposited_item": deposited,
                "pnj_zones": {
                    "Reception": {
                        "PNJPlace0": {
                            "path": "ReceptionDesk/PNJPlace0",
                            "position": [PLACE_CENTER.0, PLACE_CENTER.1, PLACE_CENTER.2],
                            "size": [PLACE_SIZE.0, PLACE_SIZE.1, PLACE_SIZE.2],
                        },
                    },
                    "TempZone": {
                        "PNJPlace0": {
                            "path": "TempZone/Place0/PNJPlace0",
                            "position": [TEMP0_CENTER.0, TEMP0_CENTER.1, TEMP0_CENTER.2],
                            "size": [TEMP_SIZE.0, TEMP_SIZE.1, TEMP_SIZE.2],
                            "target": [TEMP0_TARGET.0, TEMP0_TARGET.1, TEMP0_TARGET.2],
                        },
                        "PNJPlace1": {
                            "path": "TempZone/Place1/PNJPlace1",
                            "position": [TEMP1_CENTER.0, TEMP1_CENTER.1, TEMP1_CENTER.2],
                            "size": [TEMP_SIZE.0, TEMP_SIZE.1, TEMP_SIZE.2],
                            "target": [TEMP1_TARGET.0, TEMP1_TARGET.1, TEMP1_TARGET.2],
                        },
                    },
                },
            }),
        }
    }

    /// `depot_frame`, plus a `pnj_box_temp_zones` occupancy map (the frames
    /// replace a channel's data wholesale, so the map must travel with the
    /// rest of the depot properties).
    fn depot_frame_with_box_zones(deposited: Value, box_zones: Value) -> GorcFrame {
        let mut frame = depot_frame(deposited);
        frame
            .data
            .as_object_mut()
            .unwrap()
            .insert("pnj_box_temp_zones".into(), box_zones);
        frame
    }

    /// An item with the given scene entering the NPC's view.
    fn item_frame(uuid: &str, scenename: &str, pos: (f64, f64, f64)) -> GorcFrame {
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
                "scenename": scenename,
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

    /// A frame reparenting an item (e.g. picked up by / dropped from a player).
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

    /// The action wired to the depot above, accepting hauling boxes only.
    fn action() -> DepotStoreReceptionToTempArea {
        DepotStoreReceptionToTempArea::new(
            Some(DEPOT.to_string()),
            Some(DepotStoreReceptionToTempAreaConfig {
                boxes_type_allowed: vec!["hauling_box.tscn".into(), "small_box.tscn".into()],
            }),
        )
    }

    #[test]
    fn cannot_run_when_not_at_work() {
        let world = Arc::new(WorldView::new("self".into()));
        world.apply(&depot_frame(json!([ITEM])));
        let (ctx, _rx) = dummy_ctx(world);
        assert!(!action().can_run(&ctx));
    }

    #[test]
    fn cannot_run_without_config_or_workplace() {
        let world = Arc::new(WorldView::new("self".into()));
        world.apply(&depot_frame(json!([ITEM])));
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(!DepotStoreReceptionToTempArea::new(Some(DEPOT.into()), None).can_run(&ctx));
        let cfg = DepotStoreReceptionToTempAreaConfig {
            boxes_type_allowed: vec!["hauling_box.tscn".into()],
        };
        assert!(!DepotStoreReceptionToTempArea::new(None, Some(cfg)).can_run(&ctx));
    }

    #[test]
    fn cannot_run_when_deposit_property_absent() {
        let world = Arc::new(WorldView::new("self".into()));
        world.apply(&GorcFrame {
            data: json!({"position": {"x": 0.0, "y": 0.0, "z": 0.0}}),
            ..depot_frame(json!([]))
        });
        let (ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        assert!(!action().can_run(&ctx));
    }

    #[tokio::test]
    async fn run_consumes_the_change_so_it_is_checked_once() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!([ITEM])));
        world.apply(&item_frame(ITEM, "res://items/barrel.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, _rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        assert!(action().can_run(&ctx));
        action().run(&mut ctx).await.unwrap();
        assert!(!action().can_run(&ctx), "same value must not re-trigger");

        // A different deposit re-arms the check.
        ctx.world.apply(&depot_frame(json!([])));
        assert!(action().can_run(&ctx));
    }

    #[tokio::test]
    async fn more_than_one_item_asks_for_a_single_object() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!([ITEM, "other-item"])));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        action().run(&mut ctx).await.unwrap();

        assert_eq!(
            rx.try_recv().unwrap(),
            json!({
                "namespace": "player",
                "event": "client_action",
                "data": {
                    "action": "dialog",
                    "to": "p1",
                    "text": "Please, put only 1 object",
                    "uuid": "me",
                },
            })
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn disallowed_box_type_lists_accepted_types_without_tscn() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!([ITEM])));
        world.apply(&item_frame(ITEM, "res://items/barrel.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        action().run(&mut ctx).await.unwrap();

        assert_eq!(
            rx.try_recv().unwrap()["data"]["text"],
            json!("We only accept the type of box: hauling_box, small_box")
        );
        assert!(ctx.state.get(ACCEPTED_ITEM_KEY).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_box_is_picked_up_at_the_reception_place() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!([ITEM])));
        world.apply(&item_frame(ITEM, "res://items/hauling_box.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);

        // Feed the server's arrival notice while the run is under way, then
        // confirm the grab by reparenting the box under the NPC.
        let arriver = async {
            time::sleep(Duration::from_secs(2)).await;
            world.apply(&arrived_frame("me"));
            time::sleep(Duration::from_secs(4)).await;
            world.apply(&reparent_frame(ITEM, "me"));
        };
        let a = action();
        let (result, ()) = tokio::join!(a.run(&mut ctx), arriver);
        result.unwrap();

        // 1. Walk order to a random point inside the PNJPlace0 rectangle.
        let go = rx.try_recv().unwrap();
        assert_eq!(go["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(go["data"]["uuid"], json!(DEPOT));
        let (x, y, z) = (
            go["data"]["position"]["x"].as_f64().unwrap(),
            go["data"]["position"]["y"].as_f64().unwrap(),
            go["data"]["position"]["z"].as_f64().unwrap(),
        );
        assert!((x - PLACE_CENTER.0).abs() <= PLACE_SIZE.0 / 2.0);
        assert_eq!(y, PLACE_CENTER.1);
        assert!((z - PLACE_CENTER.2).abs() <= PLACE_SIZE.2 / 2.0);

        // 2. Arrival acknowledged.
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));

        let msgs = drain(&mut rx);

        // 3. Face order: the server pivots the NPC toward the box itself.
        assert_eq!(msgs[0]["data"]["action"], json!("npc_face_position"));
        assert_eq!(msgs[0]["data"]["uuid"], json!(ITEM));

        // 4. Head aimed from (0, 1.80, 0) at the box centre (1, 0.5, 0).
        let head = &msgs[1];
        assert_eq!(head["data"]["action"], json!("update_property"));
        let expected = (0.5f64 - HEAD_HEIGHT).atan2(1.0);
        assert!((head["data"]["head"].as_f64().unwrap() - expected).abs() < 1e-9);

        // 5. Action key pressed on the box, and nothing after it.
        assert_eq!(
            msgs[2]["data"],
            json!({"action": "action", "target_uuid": ITEM})
        );
        assert_eq!(msgs.len(), 3);

        assert_eq!(ctx.state.get(BOX_IN_HANDS_KEY), Some(json!(ITEM)));
        assert!(ctx.state.get(ACCEPTED_ITEM_KEY).is_none());
        // The deposit is consumed, but the box in hands keeps the action
        // eligible for its carry-to-temp-area step.
        assert!(a.can_run(&ctx), "the carry step must run next");
        ctx.state.remove(BOX_IN_HANDS_KEY);
        assert!(!a.can_run(&ctx), "deposit consumed by the pickup");
    }

    #[tokio::test]
    async fn box_still_in_player_hands_is_not_picked_up_yet() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&player_frame("p1"));
        world.apply(&depot_frame(json!([ITEM])));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (1.0, 0.5, 0.0)));
        // The counter saw the box while it is still in p1's hands.
        world.apply(&reparent_frame(ITEM, "p1"));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        let a = action();
        a.run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err(), "no walk order while the box is held");
        assert!(ctx.state.get(ACCEPTED_ITEM_KEY).is_none());
        assert!(a.can_run(&ctx), "must retry until the player drops the box");

        // Dropped on the counter (reparented off the player): now it proceeds.
        ctx.world.apply(&reparent_frame(ITEM, ""));
        assert!(a.can_run(&ctx));
    }

    #[tokio::test(start_paused = true)]
    async fn box_taken_back_during_the_walk_rearms_the_check() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&player_frame("p1"));
        world.apply(&depot_frame(json!([ITEM])));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);

        // While the NPC walks over, the player grabs the box again.
        let snatcher = async {
            time::sleep(Duration::from_secs(2)).await;
            world.apply(&reparent_frame(ITEM, "p1"));
            world.apply(&arrived_frame("me"));
        };
        let a = action();
        let (result, ()) = tokio::join!(a.run(&mut ctx), snatcher);
        result.unwrap();

        // Walk + ack went out, but no aim and no action key.
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        assert!(rx.try_recv().is_err());
        assert!(ctx.state.get(BOX_IN_HANDS_KEY).is_none());
        assert!(a.can_run(&ctx), "the whole pickup must be retried");
    }

    #[tokio::test(start_paused = true)]
    async fn reaching_the_place_without_server_notice_still_picks_up() {
        let world = Arc::new(WorldView::new("me".into()));
        // Already standing inside the PNJPlace0 rectangle; the server never
        // sends npc_arrived (e.g. its nav found no path to walk).
        world.apply(&self_frame("me", (0.75, 0.0, 3.95)));
        world.apply(&depot_frame(json!([ITEM])));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (1.0, 0.5, 3.5)));
        let (mut ctx, mut rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);

        let confirmer = async {
            time::sleep(Duration::from_secs(4)).await;
            world.apply(&reparent_frame(ITEM, "me"));
        };
        let a = action();
        let (result, ()) = tokio::join!(a.run(&mut ctx), confirmer);
        result.unwrap();

        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_go_to_position"));
        // No npc_arrived_ack: the fallback path has nothing to acknowledge.
        // Then the face order toward the box, the head aim, and the grab.
        let msgs = drain(&mut rx);
        assert_eq!(msgs[0]["data"]["action"], json!("npc_face_position"));
        assert_eq!(msgs[0]["data"]["uuid"], json!(ITEM));
        assert_eq!(msgs[1]["data"]["action"], json!("update_property"));
        assert!(msgs[1]["data"]["head"].is_number());
        assert_eq!(
            msgs[2]["data"],
            json!({"action": "action", "target_uuid": ITEM})
        );
        assert_eq!(msgs.len(), 3);
        assert_eq!(ctx.state.get(BOX_IN_HANDS_KEY), Some(json!(ITEM)));
    }

    #[tokio::test(start_paused = true)]
    async fn missed_arrival_still_attempts_the_grab_and_rearms_unconfirmed() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!([ITEM])));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        let a = action();
        a.run(&mut ctx).await.unwrap();

        // The walk order went out, no ack (never arrived), then the grab was
        // still attempted (face, aim, action key) — and with no confirmation
        // the check re-arms.
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_go_to_position"));
        let msgs = drain(&mut rx);
        assert_eq!(msgs[0]["data"]["action"], json!("npc_face_position"));
        assert!(msgs[1]["data"]["head"].is_number());
        assert_eq!(msgs[2]["data"]["action"], json!("action"));
        assert_eq!(msgs.len(), 3);
        assert!(ctx.state.get(ACCEPTED_ITEM_KEY).is_none());
        assert!(ctx.state.get(BOX_IN_HANDS_KEY).is_none());
        assert!(a.can_run(&ctx), "the whole pickup must be retried");
    }

    #[tokio::test]
    async fn missing_pnj_zone_leaves_the_change_pending() {
        let world = Arc::new(WorldView::new("me".into()));
        let mut depot = depot_frame(json!([ITEM]));
        depot.data.as_object_mut().unwrap().remove("pnj_zones");
        world.apply(&depot);
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        let a = action();
        a.run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err());
        assert!(ctx.state.get(ACCEPTED_ITEM_KEY).is_none());
        assert!(a.can_run(&ctx), "must retry once the zone data replicates");
    }

    #[tokio::test]
    async fn unknown_item_leaves_the_change_pending() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!([ITEM])));
        // The item itself never entered the view.
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        action().run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err());
        assert!(
            action().can_run(&ctx),
            "validation must retry once the item is replicated"
        );
    }

    #[tokio::test]
    async fn cleared_counter_is_consumed_without_a_message() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!([])));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        action().run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err());
        assert!(!action().can_run(&ctx));
    }

    #[tokio::test(start_paused = true)]
    async fn box_in_hands_is_dropped_in_a_free_temp_zone() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        // zone_0 holds a box; zone_1 is tracked but empty, so PNJPlace1 is the
        // only free place.
        world.apply(&depot_frame_with_box_zones(
            json!([]),
            json!({"zone_0": ["other-box"], "zone_1": []}),
        ));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (0.3, 1.0, 0.3)));
        world.apply(&reparent_frame(ITEM, "me"));
        let (mut ctx, mut rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(LAST_DEPOSITED_KEY, json!([]));
        ctx.state.set(BOX_IN_HANDS_KEY, json!(ITEM));

        let a = action();
        assert!(a.can_run(&ctx), "a box in hands must trigger the carry");

        // Feed the arrival notice mid-run, then confirm the drop by
        // reparenting the box off the NPC.
        let dropper = async {
            time::sleep(Duration::from_secs(2)).await;
            world.apply(&arrived_frame("me"));
            time::sleep(Duration::from_secs(4)).await;
            world.apply(&reparent_frame(ITEM, ""));
        };
        let (result, ()) = tokio::join!(a.run(&mut ctx), dropper);
        result.unwrap();

        // 1. Walk order to a random point inside the PNJPlace1 rectangle.
        let go = rx.try_recv().unwrap();
        assert_eq!(go["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(go["data"]["uuid"], json!(DEPOT));
        let (x, y, z) = (
            go["data"]["position"]["x"].as_f64().unwrap(),
            go["data"]["position"]["y"].as_f64().unwrap(),
            go["data"]["position"]["z"].as_f64().unwrap(),
        );
        assert!((x - TEMP1_CENTER.0).abs() <= TEMP_SIZE.0 / 2.0);
        assert_eq!(y, TEMP1_CENTER.1);
        assert!((z - TEMP1_CENTER.2).abs() <= TEMP_SIZE.2 / 2.0);

        // 2. Arrival acknowledged.
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));

        let msgs = drain(&mut rx);

        // 3. Face order toward the PNJPlace1 shelf target, depot-local — the
        // exact {uuid, position} contract of npc_face_position.
        let face = &msgs[0];
        assert_eq!(face["data"]["action"], json!("npc_face_position"));
        assert_eq!(face["data"]["uuid"], json!(DEPOT));
        assert_eq!(face["data"]["position"]["x"].as_f64().unwrap(), TEMP1_TARGET.0);
        assert_eq!(face["data"]["position"]["y"].as_f64().unwrap(), TEMP1_TARGET.1);
        assert_eq!(face["data"]["position"]["z"].as_f64().unwrap(), TEMP1_TARGET.2);

        // 4. Head aimed from (0, 1.80, 0) at the PNJPlace1 shelf target.
        let head = &msgs[1];
        assert_eq!(head["data"]["action"], json!("update_property"));
        let horiz = TEMP1_TARGET.0.hypot(TEMP1_TARGET.2);
        let expected = (TEMP1_TARGET.1 - HEAD_HEIGHT).atan2(horiz);
        assert!((head["data"]["head"].as_f64().unwrap() - expected).abs() < 1e-9);

        // 5. Action key pressed — with a box in hands the server drops it.
        assert_eq!(
            msgs[2]["data"],
            json!({"action": "action", "target_uuid": ITEM})
        );
        assert_eq!(msgs.len(), 3);

        assert!(ctx.state.get(BOX_IN_HANDS_KEY).is_none(), "carry consumed");
        assert!(!a.can_run(&ctx), "nothing left to do");
    }

    #[tokio::test]
    async fn full_temp_area_refuses_the_deposit_before_pickup() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        // A valid box waits on the counter, but every temp place is taken.
        world.apply(&depot_frame_with_box_zones(
            json!([ITEM]),
            json!({"zone_0": ["b0"], "zone_1": ["b1"]}),
        ));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        let a = action();
        a.run(&mut ctx).await.unwrap();

        // Both excuse lines go to the waiting player — and NO walk order: the
        // box must not be picked up.
        assert_eq!(rx.try_recv().unwrap()["data"]["text"], json!("Sorry, no more place"));
        assert_eq!(
            rx.try_recv().unwrap()["data"]["text"],
            json!("I need to put boxes away in the warehouse")
        );
        assert!(rx.try_recv().is_err());
        assert!(ctx.state.get(ACCEPTED_ITEM_KEY).is_none());
        assert!(ctx.state.get(BOX_IN_HANDS_KEY).is_none());
        assert!(!a.can_run(&ctx), "one refusal per deposit");

        // A place frees up and the SAME box is deposited again: accepted now.
        ctx.world.apply(&depot_frame_with_box_zones(
            json!([ITEM]),
            json!({"zone_0": ["b0"], "zone_1": []}),
        ));
        assert!(!a.can_run(&ctx), "unchanged deposit stays consumed");
        ctx.state.remove(LAST_DEPOSITED_KEY);
        assert!(a.can_run(&ctx));
    }

    #[test]
    fn empty_box_temp_zones_dict_keeps_every_place_free() {
        let world = Arc::new(WorldView::new("me".into()));
        // The depot replicates an EMPTY occupancy dict (no box ever entered a
        // temp zone yet): every place must read as free, exactly like an
        // absent property.
        world.apply(&depot_frame_with_box_zones(json!([]), json!({})));
        let (ctx, _rx) = dummy_ctx(world);
        let free = action().free_temp_places(&ctx).unwrap();
        assert_eq!(free.len(), 2);
    }

    #[test]
    fn vec3_accepts_array_map_and_godot_string_encodings() {
        for v in [
            json!([1.5, 2.5, 3.5]),
            json!({"x": 1.5, "y": 2.5, "z": 3.5}),
            json!("(1.5, 2.5, 3.5)"),
        ] {
            let p = vec3_from(&v).unwrap_or_else(|| panic!("failed on {v}"));
            assert_eq!((p.x, p.y, p.z), (1.5, 2.5, 3.5), "encoding {v}");
        }
        assert!(vec3_from(&json!("garbage")).is_none());
    }

    #[tokio::test]
    async fn unparseable_temp_places_retry_instead_of_refusing() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        // Vacant places whose `target` never replicated: the deposit must
        // stay pending (silent retry), NOT be refused with "no more place".
        let mut depot = depot_frame(json!([ITEM]));
        for place in ["PNJPlace0", "PNJPlace1"] {
            depot.data["pnj_zones"]["TempZone"][place]
                .as_object_mut()
                .unwrap()
                .remove("target");
        }
        world.apply(&depot);
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        let a = action();
        a.run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err(), "no dialog and no walk order");
        assert!(ctx.state.get(ACCEPTED_ITEM_KEY).is_none());
        assert!(a.can_run(&ctx), "must retry once the zone data replicates");
    }

    #[tokio::test]
    async fn no_free_temp_zone_keeps_the_box_for_a_later_try() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame_with_box_zones(
            json!([]),
            json!({"zone_0": ["b0"], "zone_1": ["b1"]}),
        ));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (0.3, 1.0, 0.3)));
        world.apply(&reparent_frame(ITEM, "me"));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(LAST_DEPOSITED_KEY, json!([]));
        ctx.state.set(BOX_IN_HANDS_KEY, json!(ITEM));

        let a = action();
        a.run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err(), "no walk order while every place is full");
        assert_eq!(ctx.state.get(BOX_IN_HANDS_KEY), Some(json!(ITEM)));
        assert!(a.can_run(&ctx), "the carry must retry once a place frees up");
    }

    #[tokio::test(start_paused = true)]
    async fn unconfirmed_drop_keeps_the_box_for_retry() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        // Only PNJPlace0 free, so the walk target (and turn goal) is fixed.
        world.apply(&depot_frame_with_box_zones(json!([]), json!({"zone_1": ["b"]})));
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (0.3, 1.0, 0.3)));
        world.apply(&reparent_frame(ITEM, "me"));
        let (mut ctx, mut rx) = dummy_ctx(Arc::clone(&world));
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(LAST_DEPOSITED_KEY, json!([]));
        ctx.state.set(BOX_IN_HANDS_KEY, json!(ITEM));

        // The box never reparents off the NPC: the drop must not count.
        let arriver = async {
            time::sleep(Duration::from_secs(2)).await;
            world.apply(&arrived_frame("me"));
        };
        let a = action();
        let (result, ()) = tokio::join!(a.run(&mut ctx), arriver);
        result.unwrap();

        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_go_to_position"));
        assert_eq!(rx.try_recv().unwrap()["data"]["action"], json!("npc_arrived_ack"));
        // The face order, then the head aim, then the drop keypress.
        let msgs = drain(&mut rx);
        assert_eq!(msgs[0]["data"]["action"], json!("npc_face_position"));
        assert_eq!(msgs[0]["data"]["uuid"], json!(DEPOT));
        assert!(msgs[1]["data"]["head"].is_number());
        assert_eq!(msgs[2]["data"]["action"], json!("action"));
        assert_eq!(msgs.len(), 3);
        assert_eq!(ctx.state.get(BOX_IN_HANDS_KEY), Some(json!(ITEM)));
        assert!(a.can_run(&ctx), "the carry must be retried");
    }

    #[tokio::test]
    async fn lost_box_abandons_the_carry_and_rearms_the_counter() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&self_frame("me", (0.0, 0.0, 0.0)));
        world.apply(&depot_frame(json!([])));
        // The box is back on the ground (server auto-drop): not in hands.
        world.apply(&item_frame(ITEM, "hauling_box.tscn", (0.3, 0.5, 0.3)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);
        ctx.state.set(LAST_DEPOSITED_KEY, json!([]));
        ctx.state.set(BOX_IN_HANDS_KEY, json!(ITEM));

        action().run(&mut ctx).await.unwrap();

        assert!(rx.try_recv().is_err(), "nothing to carry, nothing sent");
        assert!(ctx.state.get(BOX_IN_HANDS_KEY).is_none());
        assert!(
            ctx.state.get(LAST_DEPOSITED_KEY).is_none(),
            "counter check re-armed so the box can be handled again"
        );
    }

    #[tokio::test]
    async fn bare_string_deposit_counts_as_one_item() {
        let world = Arc::new(WorldView::new("me".into()));
        world.apply(&depot_frame(json!(ITEM)));
        world.apply(&item_frame(ITEM, "barrel.tscn", (1.0, 0.5, 0.0)));
        let (mut ctx, mut rx) = dummy_ctx(world);
        ctx.state.set_status(NpcStatus::AtWork);

        action().run(&mut ctx).await.unwrap();

        // The type refusal (not the "only 1 object" one) proves the bare
        // string was treated as a single item.
        assert_eq!(
            rx.try_recv().unwrap()["data"]["text"],
            json!("We only accept the type of box: hauling_box, small_box")
        );
    }
}
