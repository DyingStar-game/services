//! Helpers shared by the depot actions (`depot_store_reception_to_temp_area`,
//! `depot_store_item`): replicated-value parsing, world-state predicates,
//! head aiming, and the wait loops that turn replicated frames into
//! arrival/grab/drop confirmations.

use std::time::Duration;

use serde_json::Value;
use tokio::time;

use super::ActionContext;
use crate::horizon::messages::Vec3;

/// Eye height above the NPC's feet, matching the player rig; the head pitch is
/// computed from here toward the box centre.
pub(crate) const HEAD_HEIGHT: f64 = 1.80;

/// Poll cadence while waiting for `npc_arrived`.
pub(crate) const ARRIVAL_POLL: Duration = Duration::from_millis(200);

/// Fallback arrival gate: if the NPC's replicated position gets this close
/// (ground plane, metres) to the walk target, proceed even when the server
/// never sends `npc_arrived` (its own gate needs `<= 1.5 m` AND an empty
/// path, which a partial route can miss).
pub(crate) const ARRIVAL_FALLBACK_DIST: f64 = 0.7;

/// Poll cadence while waiting for a grab/drop confirmation.
pub(crate) const GRAB_CONFIRM_POLL: Duration = Duration::from_millis(200);

/// `pnj_box_temp_zones` key for a `TempZone` place: `PNJPlace4` → `zone_4`
/// (the digits ending the place name). `None` for a name without a trailing
/// number.
pub(crate) fn zone_key(place: &str) -> Option<String> {
    let digits = place.trim_start_matches(|c: char| !c.is_ascii_digit());
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("zone_{digits}"))
}

/// Whether `item` currently rides in someone's hands: the server reparents a
/// carried prop under its carrier, so a player uuid (the NPC's own included)
/// as `parent_id` means the box is not restable-on-the-counter yet.
pub(crate) fn carried_by_player(ctx: &ActionContext, item: &str) -> bool {
    let Some(parent) = ctx.world.objects.get(item).and_then(|o| o.parent_id.clone()) else {
        return false;
    };
    parent == ctx.world.my_uuid || ctx.world.players.contains_key(&parent)
}

/// Head pitch (radians, Godot camera convention: negative looks down) that
/// points the NPC's eyes — [`HEAD_HEIGHT`] above its feet — at the centre of
/// `item`, plus the eye-to-item distance in metres (the server's grab reach is
/// 3 m). `None` while either global position cannot be resolved.
pub(crate) fn aim_at(ctx: &ActionContext, item: &str) -> Option<(f64, f64)> {
    let target = item_global(ctx, item)?;
    pitch_toward(ctx, target)
}

/// Global position of `item`, resolved through its parent chain. `None` while
/// the item or an ancestor is not in view.
pub(crate) fn item_global(ctx: &ActionContext, item: &str) -> Option<Vec3> {
    let (pos, parent) = {
        let o = ctx.world.objects.get(item)?;
        (o.position, o.parent_id.clone())
    };
    ctx.world.resolve_global(pos, parent.as_deref())
}

/// Same as [`aim_at`], but toward `point` expressed in `parent`'s local frame
/// (e.g. a temp-zone `target` or a shelf slot, local to the depot). Computed
/// entirely in `parent`'s frame — the NPC's position is re-expressed there
/// via its own parent chain — so it keeps working even when `parent`'s
/// ancestors (its planet, …) never replicated into view and a root-frame
/// resolution would fail.
pub(crate) fn aim_at_local(ctx: &ActionContext, parent: &str, point: Vec3) -> Option<(f64, f64)> {
    let npc = ctx.world.my_position_in(parent)?;
    let dy = point.y - (npc.y + HEAD_HEIGHT);
    let (dx, dz) = (point.x - npc.x, point.z - npc.z);
    let horiz = dx.hypot(dz);
    Some((dy.atan2(horiz), horiz.hypot(dy)))
}

/// Head pitch and eye distance from the NPC's eyes to the global `target`.
pub(crate) fn pitch_toward(ctx: &ActionContext, target: Vec3) -> Option<(f64, f64)> {
    let npc = ctx.world.my_global_position()?;
    let dy = target.y - (npc.y + HEAD_HEIGHT);
    let (dx, dz) = (target.x - npc.x, target.z - npc.z);
    let horiz = dx.hypot(dz);
    Some((dy.atan2(horiz), horiz.hypot(dy)))
}

/// `v` rotated by `yaw` radians around the +Y (up) axis, Godot convention:
/// `Vector3(1, 0, 0).rotated(Vector3.UP, PI / 2) == Vector3(0, 0, -1)`.
/// Turns a point local to a yawed object (a shelf) into its parent's frame.
pub(crate) fn rotate_y(v: Vec3, yaw: f64) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    Vec3 {
        x: v.x * c + v.z * s,
        y: v.y,
        z: -v.x * s + v.z * c,
    }
}

/// How a wait for the walk target ended.
pub(crate) enum ArrivalOutcome {
    /// The server flagged `npc_arrived` (needs an ack).
    Notified,
    /// No server notice, but the NPC's own position reached the target
    /// (within [`ARRIVAL_FALLBACK_DIST`]).
    CloseEnough,
    TimedOut,
}

/// Ground-plane distance (metres) from the NPC to `target` (local to
/// `depot`), computed in the depot's frame — robust to the depot's own
/// ancestors being out of view. `None` while the NPC cannot be located
/// relative to the depot.
pub(crate) fn ground_distance_to(ctx: &ActionContext, depot: &str, target: Vec3) -> Option<f64> {
    let npc = ctx.world.my_position_in(depot)?;
    Some((target.x - npc.x).hypot(target.z - npc.z))
}

/// Wait until the box replicates as parented under this NPC (grab confirmed),
/// or `timeout` elapses.
pub(crate) async fn wait_for_box_in_hands(
    ctx: &ActionContext,
    item: &str,
    timeout: Duration,
) -> bool {
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
pub(crate) async fn wait_for_box_released(
    ctx: &ActionContext,
    item: &str,
    timeout: Duration,
) -> bool {
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
pub(crate) async fn wait_for_arrival(
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
pub(crate) fn vec3_from(v: &Value) -> Option<Vec3> {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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

    #[test]
    fn rotate_y_at_zero_is_identity() {
        let v = Vec3 { x: 1.5, y: 2.5, z: -3.5 };
        let r = rotate_y(v, 0.0);
        assert_eq!((r.x, r.y, r.z), (1.5, 2.5, -3.5));
    }

    #[test]
    fn rotate_y_follows_the_godot_convention() {
        // Vector3(1, 0, 0).rotated(Vector3.UP, PI / 2) == Vector3(0, 0, -1)
        let r = rotate_y(Vec3 { x: 1.0, y: 0.0, z: 0.0 }, std::f64::consts::FRAC_PI_2);
        assert!(r.x.abs() < 1e-12);
        assert_eq!(r.y, 0.0);
        assert!((r.z + 1.0).abs() < 1e-12);
    }

    #[test]
    fn rotate_y_round_trips_at_an_arbitrary_yaw() {
        let v = Vec3 { x: 0.75, y: 1.2, z: -2.3 };
        let yaw = 0.83;
        let back = rotate_y(rotate_y(v, yaw), -yaw);
        assert!((back.x - v.x).abs() < 1e-12);
        assert_eq!(back.y, v.y);
        assert!((back.z - v.z).abs() < 1e-12);
    }
}
