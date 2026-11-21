//! Traffic logic of an NPC driver (ADR services/0012): given the things near
//! the road, in route coordinates, decide each tick how the controller should
//! drive — keep to the right lane, follow, overtake, steer round, give way,
//! stop, or (after waiting) leave the road to get round a blockage.
//!
//! Pure: no I/O, no clock of its own (`now` is passed in). Coordinates are the
//! route's: `s` metres along it, `d` metres across (positive = left).

use super::{
    controller::{Controller, Manoeuvre, RoutePt, Tuning},
    lanes::Lanes,
    obstacles::{Kind, Obstacle},
    offroad,
};
use crate::{horizon::messages::Vec3, world_data::geo};

/// The NPC truck: its exported collision footprint (`truck.json`), 2.19 m wide
/// (mirrors included) and 3.65 m long.
pub const TRUCK_HALF_W: f64 = 1.09;
pub const TRUCK_HALF_L: f64 = 1.825;
/// Gap kept sideways between the truck and anything it passes.
const CLEARANCE_M: f64 = 0.5;
/// Kept from the edges of the paved road by the truck's side. Small: the
/// cruise lane itself keeps the side only 0.15 m from the edge (lanes.rs).
const EDGE_M: f64 = 0.1;
/// Lateral freedom off the road (the centreline of an off-road leg).
const OFFROAD_HALF_M: f64 = 4.0;
/// Bumper-to-bumper distance to stop short of something to give way to.
const STOP_GAP_M: f64 = 5.0;
/// … and short of a blockage, leaving room to steer round it afterwards (the
/// truck's wheels turn slowly).
const BLOCKED_GAP_M: f64 = 10.0;
/// Speed of an off-road detour round a blockage.
pub const DETOUR_KMH: f64 = 8.0;
/// Things this far from the blockage are kept clear of on the detour too.
const DETOUR_KEEP_CLEAR_M: f64 = 40.0;
/// Following distance: this plus `FOLLOW_S` × speed.
const FOLLOW_GAP_M: f64 = 5.0;
const FOLLOW_S: f64 = 1.5;
/// A vehicle ahead this much slower than the road is worth overtaking.
const OVERTAKE_DV_MPS: f64 = 3.0;
/// Seconds of lateral motion swept into an obstacle's band (people crossing).
const SWEEP_S: f64 = 2.0;
/// Margin on top of the time to pass, against oncoming traffic, metres.
const ONCOMING_MARGIN_M: f64 = 20.0;
/// How far a vehicle can be seen: the game replicates vehicles within 200 m
/// (`vehicle_def.json`, zone 0). Passing into the oncoming lane needs a clear
/// stretch no longer than that, or a car beyond sight could be met head-on.
const VISIBILITY_M: f64 = 200.0;
/// How long a blockage is waited out before getting round it off the road:
/// a minute when someone is there who could clear it (a person, or a vehicle
/// with someone aboard, within `ATTENDED_M` of it), honking at `HONK_AT_S`;
/// a few seconds when nobody is.
pub const WAIT_ATTENDED_S: f64 = 60.0;
pub const WAIT_UNATTENDED_S: f64 = 5.0;
const ATTENDED_M: f64 = 25.0;
const HONK_AT_S: [f64; 2] = [10.0, 40.0];
/// A cleared blockage must stay clear this long before driving on.
const CLEAR_HOLD_S: f64 = 1.0;
/// Moving slower than this (m/s, along the road) = standing.
const STANDING_MPS: f64 = 1.0;

/// What the driver is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Nothing in the way: cruise in the lane.
    Cruise,
    /// Behind a slower vehicle, at a safe distance.
    Follow,
    /// Passing a slower vehicle in the overtaking lane.
    Overtake,
    /// Steering round something standing on the road.
    Avoid,
    /// Giving way: oncoming vehicle in the lane, someone crossing, or waiting
    /// for the other lane to clear before passing.
    Yield,
    /// The road is blocked: stopped, waiting.
    Blocked,
    /// Off the road, round a blockage.
    Bypass,
}

/// A thing near the road, in route coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    pub id: String,
    pub kind: Kind,
    pub s: f64,
    pub d: f64,
    /// Half its extent along and across the road (its box, turned).
    pub half_along: f64,
    pub half_across: f64,
    /// Its velocity along (+ = our way) and across (+ = left) the road.
    pub v_along: f64,
    pub v_across: f64,
    /// Someone is there (a person, or a vehicle with someone aboard).
    pub occupied: bool,
}

impl Seen {
    fn front(&self) -> f64 {
        self.s - self.half_along
    }
    fn back(&self) -> f64 {
        self.s + self.half_along
    }
    /// Its band across the road, swept over the next seconds of sideways
    /// motion (someone walking across the road).
    fn band(&self) -> (f64, f64) {
        let to = self.d + self.v_across * SWEEP_S;
        (self.d.min(to) - self.half_across, self.d.max(to) + self.half_across)
    }
    /// Whether the truck centred at `d` would touch it.
    fn blocks(&self, d: f64) -> bool {
        let (lo, hi) = self.band();
        let reach = TRUCK_HALF_W + CLEARANCE_M;
        hi > d - reach && lo < d + reach
    }
}

/// Project an obstacle onto the controller's route, measured from `vehicle`.
/// `None` when it is not near the stretch ahead.
pub fn seen(ctrl: &Controller, vehicle: Vec3, ob: &Obstacle, ahead_m: f64) -> Option<Seen> {
    let spot = ctrl.project(vehicle, ob.pos, ahead_m, 30.0)?;
    let tan = geo::Tangent::at(vehicle);
    let (ux, uy) = spot.dir;
    let along = |v: (f64, f64)| v.0 * ux + v.1 * uy;
    let across = |v: (f64, f64)| -v.0 * uy + v.1 * ux;
    let a = tan.project_dir(ob.axis);
    let n = a.0.hypot(a.1);
    let (ca, cn) = if n < 1e-6 { (1.0, 0.0) } else { (along(a) / n, across(a) / n) };
    let v = tan.project_dir(ob.vel);
    Some(Seen {
        id: ob.id.clone(),
        kind: ob.kind,
        s: spot.s,
        d: spot.d,
        half_along: ca.abs() * ob.half_l + cn.abs() * ob.half_w,
        half_across: cn.abs() * ob.half_l + ca.abs() * ob.half_w,
        v_along: along(v),
        v_across: across(v),
        occupied: ob.occupied,
    })
}

/// The truck, in route coordinates.
#[derive(Debug, Clone, Copy)]
pub struct Me {
    pub s: f64,
    pub d: f64,
    /// Forward speed, m/s.
    pub v: f64,
    /// The road's speed limit here, m/s.
    pub cruise_mps: f64,
}

/// The road along the route: its lanes and the truck's lane offset.
pub trait RoadView {
    fn lanes_at(&self, s: f64) -> Lanes;
    fn base_offset_at(&self, s: f64) -> f64;
}

impl RoadView for Controller {
    fn lanes_at(&self, s: f64) -> Lanes {
        Controller::lanes_at(self, s)
    }
    fn base_offset_at(&self, s: f64) -> f64 {
        Controller::base_offset_at(self, s)
    }
}

/// Getting round a blockage off the road: from where the truck is to the route
/// at `rejoin_s`, round the things in `blockers`, passing on `side`.
#[derive(Debug, Clone, PartialEq)]
pub struct BypassRequest {
    pub blockers: Vec<String>,
    /// From the front of the blockage to past its back, along the route.
    pub s_from: f64,
    pub s_to: f64,
    pub rejoin_s: f64,
    /// +1 = pass on the left, −1 = on the right.
    pub side: f64,
}

/// What the controller should do this tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub mode: Mode,
    pub manoeuvre: Option<Manoeuvre>,
    pub speed_cap: Option<f64>,
    pub stop_at: Option<f64>,
    /// Set once, when a blockage has been waited out: plan a way round.
    pub bypass: Option<BypassRequest>,
    /// The thing that decided it, and the bumper gap to it.
    pub lead: Option<(String, f64)>,
    /// Sound the horn now (waiting at a blockage someone could clear).
    pub honk: bool,
    /// Waiting at a blockage: whether someone is there, and how long the wait.
    pub attended: Option<bool>,
}

/// The driver's state between ticks.
#[derive(Debug, Default)]
pub struct Traffic {
    mode: Option<Mode>,
    manoeuvre: Option<Manoeuvre>,
    blocked_since: Option<f64>,
    blocked_stop: Option<f64>,
    clear_since: Option<f64>,
    bypass_sent: bool,
    /// While off the road round a blockage: where the detour ends.
    bypass_end: Option<f64>,
    /// Detours tried round the current blockage.
    pub bypass_attempts: u32,
    /// Horn blasts given at the current blockage.
    honks: usize,
}

impl Traffic {
    pub fn mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Cruise)
    }

    /// Seconds stopped at the current blockage.
    pub fn blocked_s(&self, now: f64) -> f64 {
        self.blocked_since.map(|t| now - t).unwrap_or(0.0)
    }

    /// The route changed (re-planned, or a detour round a blockage ending at
    /// `bypass_end` on the new route): every `s` held is void.
    pub fn new_route(&mut self, bypass_end: Option<f64>) {
        self.manoeuvre = None;
        self.blocked_since = None;
        self.blocked_stop = None;
        self.clear_since = None;
        self.bypass_sent = false;
        self.honks = 0;
        self.bypass_end = bypass_end;
        self.mode = bypass_end.map(|_| Mode::Bypass);
        if bypass_end.is_none() {
            self.bypass_attempts = 0;
        }
    }

    /// The detour planned for a [`BypassRequest`] could not be driven (or
    /// planned): wait again, then try the other side.
    pub fn bypass_failed(&mut self) {
        self.bypass_attempts += 1;
        self.bypass_sent = false;
    }

    pub fn decide(&mut self, now: f64, me: Me, seen: &[Seen], road: &dyn RoadView) -> Decision {
        let horizon = (me.v * me.v / 3.0 + 30.0).max(60.0);
        // Off the road round a blockage, until back on the route past it.
        if let Some(end) = self.bypass_end {
            if me.s > end {
                self.bypass_end = None;
                self.bypass_attempts = 0;
                self.mode = None;
            }
        }
        let path = |s: f64, m: &Option<Manoeuvre>| {
            let base = road.base_offset_at(s);
            m.map(|m| base + (m.d_m - base) * m.weight(s)).unwrap_or(base)
        };
        // A finished manoeuvre is dropped.
        if self.manoeuvre.is_some_and(|m| me.s > m.s_out1) {
            self.manoeuvre = None;
        }
        let near: Vec<&Seen> = seen
            .iter()
            .filter(|o| o.back() > me.s - TRUCK_HALF_L && o.front() < me.s + horizon)
            .filter(|o| {
                let l = road.lanes_at(o.s);
                let half = if l.is_road() { l.half_width_m } else { OFFROAD_HALF_M };
                o.d.abs() - o.half_across < half + 3.0
            })
            .collect();
        let gap = |o: &Seen| o.front() - (me.s + TRUCK_HALF_L);
        // The nearest thing ahead on the path being followed.
        let lead = near
            .iter()
            .copied()
            .filter(|o| o.back() > me.s + TRUCK_HALF_L * 0.5 && o.blocks(path(o.s.max(me.s), &self.manoeuvre)))
            .min_by(|a, b| gap(a).total_cmp(&gap(b)));

        let mut out =
            Decision { mode: Mode::Cruise, manoeuvre: None, speed_cap: None, stop_at: None, bypass: None, lead: None, honk: false, attended: None };
        let Some(lead) = lead else {
            return self.clear(now, me, out);
        };
        out.lead = Some((lead.id.clone(), gap(lead)));
        let in_bypass = self.bypass_end.is_some();

        // ── Someone crossing: wait for them ──
        if lead.kind == Kind::Person && lead.v_across.abs() > 0.3 {
            return self.hold(now, out, Mode::Yield, lead.front() - 3.0 - TRUCK_HALF_L, false);
        }
        // ── Oncoming in our lane: squeeze right, slow, stop if not enough ──
        if lead.v_along < -STANDING_MPS {
            let lanes = road.lanes_at(lead.s);
            let (lo, _) = lanes.centre_range(TRUCK_HALF_W, EDGE_M, OFFROAD_HALF_M);
            if !lead.blocks(lo) {
                let m = Manoeuvre { s_in0: me.s, s_in1: me.s + 8.0, s_out0: lead.back() + 2.0 * TRUCK_HALF_L + 3.0, s_out1: lead.back() + 2.0 * TRUCK_HALF_L + 13.0, d_m: lo };
                self.manoeuvre = Some(m);
                out.manoeuvre = Some(m);
                out.speed_cap = Some(me.v.min(5.0));
                out.mode = Mode::Yield;
                self.mode = Some(Mode::Yield);
                return out;
            }
            return self.hold(now, out, Mode::Yield, me.s + (gap(lead) - 3.0).max(0.0), false);
        }

        // ── Moving our way: follow, or overtake when much slower ──
        let moving = lead.v_along > STANDING_MPS;
        if moving && lead.v_along >= me.cruise_mps - OVERTAKE_DV_MPS {
            return self.follow(now, me, lead, out);
        }

        // ── Standing (or slow): find room beside it ──
        let span = (lead.front() - 2.0, lead.back() + 2.0);
        let lanes = road.lanes_at(lead.s);
        let range = lanes.centre_range(TRUCK_HALF_W, EDGE_M, OFFROAD_HALF_M);
        let others: Vec<&Seen> = near
            .iter()
            .copied()
            .filter(|o| o.back() > span.0 && o.front() < span.1 && (moving || o.v_along.abs() <= STANDING_MPS || o.id == lead.id))
            .collect();
        let want = road.base_offset_at(lead.s);
        let Some(target) = free_offset(range, &others, want) else {
            if moving {
                return self.follow(now, me, lead, out);
            }
            return self.blocked(now, me, lead, &others, &near, range, out, in_bypass);
        };
        // Into the oncoming half of a two-way road: only with nobody coming.
        let crosses = lanes.overtake_oncoming && target + TRUCK_HALF_W > 0.0;
        let pass_end = lead.back() + 2.0 * TRUCK_HALF_L + 4.0;
        if crosses {
            let v_pass = me.cruise_mps.min(me.v.max(3.0)).max(3.0);
            let rel = (v_pass - lead.v_along.max(0.0)).max(1.0);
            let t_pass = (pass_end - me.s + lead.v_along.max(0.0) * 2.0) / rel;
            let need = pass_end - me.s + lead.v_along.max(0.0) * t_pass;
            // A car coming at the road's speed, still out of sight, must not
            // reach us before we are back: else follow (or wait) instead.
            if need + me.cruise_mps * t_pass + ONCOMING_MARGIN_M > VISIBILITY_M {
                if moving {
                    return self.follow(now, me, lead, out);
                }
                if self.manoeuvre.is_none_or(|m| (m.d_m - target).abs() >= 0.3) && need > VISIBILITY_M / 2.0 {
                    return self.hold(now, out, Mode::Yield, lead.front() - STOP_GAP_M - TRUCK_HALF_L, false);
                }
            }
            // Everything seen, not only what is within the horizon: oncoming
            // traffic matters from further away.
            let coming = seen.iter().any(|o| {
                o.v_along < -0.5 && o.blocks(target) && o.s > me.s && o.s - me.s < need + (-o.v_along) * t_pass + ONCOMING_MARGIN_M
            });
            if coming && !self.manoeuvre.is_some_and(|m| (m.d_m - target).abs() < 0.3 && me.s > m.s_in1) {
                if moving {
                    return self.follow(now, me, lead, out);
                }
                return self.hold(now, out, Mode::Yield, lead.front() - STOP_GAP_M - TRUCK_HALF_L, false);
            }
        }
        // Steer round: blend over a distance that grows with speed, reaching
        // the offset before the obstacle when there is room to.
        let change = (me.v * 2.0).clamp(10.0, 30.0);
        let room = gap(lead) - 1.0;
        let keep = self.manoeuvre.filter(|m| (m.d_m - target).abs() < 0.3);
        let (s_in0, s_in1) = match keep {
            Some(m) => (m.s_in0, m.s_in1),
            None => (me.s, me.s + change.min(room.max(6.0))),
        };
        let s_out0 = keep.map(|m| m.s_out0).unwrap_or(f64::MIN).max(pass_end + lead.v_along.max(0.0) * 3.0);
        let m = Manoeuvre { s_in0, s_in1, s_out0, s_out1: s_out0 + change, d_m: target };
        self.manoeuvre = Some(m);
        out.manoeuvre = Some(m);
        out.mode = if moving { Mode::Overtake } else { Mode::Avoid };
        // Not across yet and close: slow to a crawl so the wheels can bring it round.
        if (me.d - target).abs() > 0.5 && gap(lead) < 10.0 {
            out.speed_cap = Some(2.0);
            if gap(lead) < 2.0 {
                out.stop_at = Some(lead.front() - 1.5 - TRUCK_HALF_L);
            }
        }
        if in_bypass {
            out.mode = Mode::Bypass;
        }
        self.blocked_since = None;
        self.blocked_stop = None;
        self.mode = Some(out.mode);
        out
    }

    /// Nothing in the way. After a stop, drive on only once clear for a moment.
    fn clear(&mut self, now: f64, me: Me, mut out: Decision) -> Decision {
        if let Some(stop) = self.blocked_stop {
            let since = *self.clear_since.get_or_insert(now);
            if now - since < CLEAR_HOLD_S {
                out.stop_at = Some(stop);
                out.mode = self.mode();
                return out;
            }
        }
        self.blocked_since = None;
        self.blocked_stop = None;
        self.clear_since = None;
        self.bypass_sent = false;
        self.honks = 0;
        out.manoeuvre = self.manoeuvre.filter(|m| me.s <= m.s_out1);
        out.mode = if self.bypass_end.is_some() { Mode::Bypass } else { Mode::Cruise };
        self.mode = Some(out.mode);
        out
    }

    fn follow(&mut self, _now: f64, me: Me, lead: &Seen, mut out: Decision) -> Decision {
        let gap = lead.front() - (me.s + TRUCK_HALF_L);
        let want = FOLLOW_GAP_M + FOLLOW_S * me.v;
        out.speed_cap = Some((lead.v_along + 0.5 * (gap - want)).max(0.0));
        if gap < STOP_GAP_M * 0.6 {
            out.stop_at = Some(lead.front() - 3.0 - TRUCK_HALF_L);
        }
        out.manoeuvre = self.manoeuvre.filter(|m| me.s <= m.s_out1);
        out.mode = Mode::Follow;
        self.blocked_since = None;
        self.blocked_stop = None;
        self.mode = Some(Mode::Follow);
        out
    }

    /// Stop at `stop` and wait. The wait at a blockage counts while
    /// `waiting` (stopped in front of it); giving way does not count.
    fn hold(&mut self, now: f64, mut out: Decision, mode: Mode, stop: f64, waiting: bool) -> Decision {
        out.stop_at = Some(stop);
        out.mode = mode;
        out.manoeuvre = self.manoeuvre;
        self.clear_since = None;
        self.blocked_stop = Some(stop);
        if waiting {
            self.blocked_since.get_or_insert(now);
        } else if mode != Mode::Blocked {
            self.blocked_since = None;
        }
        self.mode = Some(mode);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn blocked(
        &mut self,
        now: f64,
        me: Me,
        lead: &Seen,
        others: &[&Seen],
        near: &[&Seen],
        range: (f64, f64),
        out: Decision,
        in_bypass: bool,
    ) -> Decision {
        let stop = lead.front() - BLOCKED_GAP_M - TRUCK_HALF_L;
        let stop = self.blocked_stop.filter(|s| (s - stop).abs() < 2.0).unwrap_or(stop);
        // The wait is counted once stopped in front of it, not from afar.
        let waiting = me.v.abs() < 0.3 && me.s >= stop - 3.0;
        let mut out = self.hold(now, out, Mode::Blocked, stop.max(me.s.min(stop)), waiting);
        // Someone who could clear it: wait for them, honking; else go round soon.
        let attended = near.iter().any(|o| o.occupied && others.iter().any(|b| box_gap(o, b) <= ATTENDED_M));
        let waited = self.blocked_s(now);
        out.attended = Some(attended);
        if attended && self.honks < HONK_AT_S.len() && waited >= HONK_AT_S[self.honks] {
            out.honk = true;
            self.honks += 1;
        }
        let wait = if attended { WAIT_ATTENDED_S } else { WAIT_UNATTENDED_S };
        if !in_bypass && !self.bypass_sent && waited >= wait && self.bypass_attempts < 2 {
            // Pass on the side the blockage leaves more room on (and the other
            // side on the second try).
            let lo = others.iter().map(|o| o.band().0).fold(f64::INFINITY, f64::min);
            let hi = others.iter().map(|o| o.band().1).fold(f64::NEG_INFINITY, f64::max);
            let first = if range.1 - hi >= lo - range.0 { 1.0 } else { -1.0 };
            let side = if self.bypass_attempts == 0 { first } else { -first };
            let s_from = others.iter().map(|o| o.front()).fold(f64::INFINITY, f64::min);
            let s_to = others.iter().map(|o| o.back()).fold(f64::NEG_INFINITY, f64::max);
            out.bypass = Some(BypassRequest {
                blockers: others.iter().map(|o| o.id.clone()).collect(),
                s_from,
                s_to,
                rejoin_s: s_to + 15.0,
                side,
            });
            self.bypass_sent = true;
        }
        out
    }
}

/// Clearance of the off-road detour round a blockage, and its planning cell.
/// The path is the truck's CENTRE, and the truck turns while it goes round a
/// corner it approaches at an angle: its nose reaches ~1.7 m ahead of the
/// centre, so half its length plus half its width, plus pure pursuit cutting
/// a little inside.
const DETOUR_CLEARANCE_M: f64 = 2.6;
const DETOUR_CELL_M: f64 = 0.5;
/// Points of the detour are spaced this much (the controller interpolates).
const DETOUR_STEP_M: f64 = 2.0;

/// A route round a blockage: from `start` (the truck), off the road round the
/// boxes of `req.blockers` on `req.side` (a virtual wall closes the other
/// side), back onto the route at `req.rejoin_s`, then the rest of the route.
/// `extra` are fixed footprints to keep clear of too (village buildings).
/// Returns the new route and the length of its off-road part, or `None` when
/// no way round is found.
pub fn detour(
    ctrl: &Controller,
    req: &BypassRequest,
    obstacles: &[Obstacle],
    extra: &[offroad::Obstacle],
    start: Vec3,
    offroad_kmh: f64,
) -> Option<(Vec<RoutePt>, f64)> {
    let radius = geo::length(start);
    let mut polys: Vec<offroad::Obstacle> = extra.to_vec();
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let blockage = ctrl.position_at((req.s_from + req.s_to) / 2.0, 0.0, radius);
    // Every box near the blockage — not only the blockers: what stands by the
    // road is where the detour goes.
    for ob in obstacles.iter().filter(|o| req.blockers.contains(&o.id) || geo::arc_m(o.pos, blockage, radius) < DETOUR_KEEP_CLEAR_M) {
        let up = geo::normalized(ob.pos);
        let side = geo::normalized(geo::cross(up, ob.axis));
        let fwd = geo::normalized(geo::cross(side, up));
        let corner = |a: f64, b: f64| geo::add(ob.pos, geo::add(geo::scale(fwd, a * ob.half_l), geo::scale(side, b * ob.half_w)));
        polys.push(vec![corner(1.0, 1.0), corner(1.0, -1.0), corner(-1.0, -1.0), corner(-1.0, 1.0)]);
        if req.blockers.contains(&ob.id) {
            if let Some(spot) = ctrl.project(start, ob.pos, 200.0, 40.0) {
                lo = lo.min(spot.d);
                hi = hi.max(spot.d);
            }
        }
    }
    if !lo.is_finite() {
        return None;
    }
    // Close the other side: a wall from the blockage's middle out 40 m.
    let mid = (lo + hi) / 2.0;
    let (d0, d1) = if req.side > 0.0 { (mid - 40.0, mid) } else { (mid, mid + 40.0) };
    let (s0, s1) = (req.s_from - 1.0, req.s_to + 1.0);
    polys.push(vec![
        ctrl.position_at(s0, d0, radius),
        ctrl.position_at(s1, d0, radius),
        ctrl.position_at(s1, d1, radius),
        ctrl.position_at(s0, d1, radius),
    ]);
    let goal = ctrl.position_at(req.rejoin_s, ctrl.base_offset_at(req.rejoin_s), radius);
    let path = offroad::plan(start, goal, &polys, DETOUR_CLEARANCE_M, DETOUR_CELL_M)?;

    // Densify the detour, then carry on along the old route from the rejoin.
    let mut pts: Vec<RoutePt> = Vec::new();
    let mut s = 0.0;
    for w in path.windows(2) {
        let len = geo::arc_m(w[0], w[1], radius);
        let n = ((len / DETOUR_STEP_M).ceil() as usize).max(1);
        for i in 0..n {
            let t = i as f64 / n as f64;
            let unit = geo::normalized(geo::add(geo::scale(w[0], 1.0 - t), geo::scale(w[1], t)));
            pts.push(RoutePt { unit, s: s + len * t, speed_kmh: offroad_kmh, half_width_m: 0.0, lanes: 0 });
        }
        s += len;
    }
    let detour_m = s;
    for p in ctrl.points_from(req.rejoin_s) {
        let ds = p.s - req.rejoin_s;
        if ds > 0.05 || pts.is_empty() {
            pts.push(RoutePt { s: detour_m + ds, ..p });
        }
    }
    Some((pts, detour_m))
}

/// Distance between two things on the road, edge to edge (route coordinates).
fn box_gap(a: &Seen, b: &Seen) -> f64 {
    let ds = ((a.s - b.s).abs() - a.half_along - b.half_along).max(0.0);
    let dd = ((a.d - b.d).abs() - a.half_across - b.half_across).max(0.0);
    ds.hypot(dd)
}

/// Controller tuning for a detour: a short lookahead, so the truck stays on
/// the planned path round the corners instead of cutting inside them.
pub fn detour_tuning(base: Tuning) -> Tuning {
    Tuning { lookahead_min_m: 4.0, lookahead_k_s: 1.0, ..base }
}

/// The offset closest to `want` within `range` where the truck clears every
/// one of `others` (each widened by the truck's half width and the clearance).
fn free_offset(range: (f64, f64), others: &[&Seen], want: f64) -> Option<f64> {
    let reach = TRUCK_HALF_W + CLEARANCE_M;
    let mut blocked: Vec<(f64, f64)> = others
        .iter()
        .map(|o| {
            let (lo, hi) = o.band();
            (lo - reach, hi + reach)
        })
        .collect();
    blocked.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Free intervals = range minus the union of the blocked ones.
    let mut free = Vec::new();
    let mut cursor = range.0;
    for (lo, hi) in blocked {
        if lo > cursor {
            free.push((cursor, lo.min(range.1)));
        }
        cursor = cursor.max(hi);
        if cursor >= range.1 {
            break;
        }
    }
    if cursor < range.1 {
        free.push((cursor, range.1));
    }
    free.into_iter()
        .filter(|(a, b)| b >= a)
        .map(|(a, b)| want.clamp(a, b))
        .min_by(|x, y| (x - want).abs().total_cmp(&(y - want).abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::driving::lanes;

    /// A straight two-lane road (6 m) all along, cruising 1.75 m right of its centreline.
    struct TwoLane;
    impl RoadView for TwoLane {
        fn lanes_at(&self, _s: f64) -> Lanes {
            lanes::lanes(3.0, 2)
        }
        fn base_offset_at(&self, _s: f64) -> f64 {
            -1.75
        }
    }

    fn me(s: f64, v: f64) -> Me {
        Me { s, d: -1.75, v, cruise_mps: 50.0 / 3.6 }
    }

    fn truck(id: &str, s: f64, d: f64, v: f64) -> Seen {
        Seen { id: id.into(), kind: Kind::Vehicle, s, d, half_along: 1.8, half_across: 0.9, v_along: v, v_across: 0.0, occupied: false }
    }

    #[test]
    fn cruises_in_the_right_lane_on_an_empty_road() {
        let mut t = Traffic::default();
        let d = t.decide(0.0, me(0.0, 10.0), &[], &TwoLane);
        assert_eq!(d.mode, Mode::Cruise);
        assert!(d.manoeuvre.is_none() && d.speed_cap.is_none() && d.stop_at.is_none());
    }

    #[test]
    fn steers_round_a_parked_truck_through_the_free_lane_then_back() {
        // Image 1: a truck stopped in our lane, the other lane free.
        let mut t = Traffic::default();
        let parked = truck("parked", 60.0, -1.75, 0.0);
        let d = t.decide(0.0, me(0.0, 10.0), std::slice::from_ref(&parked), &TwoLane);
        assert_eq!(d.mode, Mode::Avoid);
        let m = d.manoeuvre.unwrap();
        assert!(m.d_m > 0.7, "into the left half: {}", m.d_m);
        assert!(m.s_in1 < parked.front(), "across before reaching it");
        assert!(m.s_out0 > parked.back() + 2.0 * TRUCK_HALF_L, "back only once past it");
        // Once past it, the manoeuvre ends and the truck is back to cruising.
        let d = t.decide(10.0, me(m.s_out1 + 1.0, 10.0), &[parked], &TwoLane);
        assert_eq!(d.mode, Mode::Cruise);
        assert!(d.manoeuvre.is_none());
    }

    #[test]
    fn waits_for_oncoming_traffic_before_passing() {
        let mut t = Traffic::default();
        let parked = truck("parked", 60.0, -1.75, 0.0);
        let coming = truck("coming", 150.0, 1.75, -12.0);
        let d = t.decide(0.0, me(0.0, 10.0), &[parked.clone(), coming.clone()], &TwoLane);
        assert_eq!(d.mode, Mode::Yield);
        assert!(d.stop_at.unwrap() < parked.front() - TRUCK_HALF_L);
        // It has gone by: pass.
        let gone = truck("coming", -30.0, 1.75, -12.0);
        let d = t.decide(8.0, me(50.0, 0.0), &[parked, gone], &TwoLane);
        assert_eq!(d.mode, Mode::Avoid);
    }

    #[test]
    fn follows_a_slightly_slower_vehicle() {
        let mut t = Traffic::default();
        let lead = truck("lead", 30.0, -1.75, 50.0 / 3.6 - 2.0);
        let d = t.decide(0.0, me(0.0, 13.0), &[lead], &TwoLane);
        assert_eq!(d.mode, Mode::Follow);
        assert!(d.manoeuvre.is_none());
        assert!(d.speed_cap.unwrap() < 13.0);
    }

    #[test]
    fn overtakes_a_much_slower_vehicle_when_the_other_lane_is_free() {
        let mut t = Traffic::default();
        let lead = truck("tractor", 30.0, -1.75, 5.0);
        let d = t.decide(0.0, me(0.0, 12.0), &[lead], &TwoLane);
        assert_eq!(d.mode, Mode::Overtake);
        assert!(d.manoeuvre.unwrap().d_m > 0.7);
    }

    #[test]
    fn stops_for_someone_crossing() {
        let mut t = Traffic::default();
        let walker = Seen { id: "p".into(), kind: Kind::Person, s: 25.0, d: 2.5, half_along: 0.3, half_across: 0.3, v_along: 0.0, v_across: -1.4, occupied: true };
        let d = t.decide(0.0, me(0.0, 10.0), &[walker], &TwoLane);
        assert_eq!(d.mode, Mode::Yield);
        assert!(d.stop_at.unwrap() < 25.0 - TRUCK_HALF_L);
    }

    fn blockage(occupied: bool) -> Vec<Seen> {
        // Image 2: a truck across the whole road, a crate filling the rest.
        vec![
            Seen { id: "across".into(), kind: Kind::Vehicle, s: 40.0, d: -0.5, half_along: 0.9, half_across: 2.4, v_along: 0.0, v_across: 0.0, occupied },
            Seen { id: "crate".into(), kind: Kind::Prop, s: 40.5, d: 2.2, half_along: 0.6, half_across: 0.6, v_along: 0.0, v_across: 0.0, occupied: false },
        ]
    }

    #[test]
    fn an_unattended_blockage_is_bypassed_after_a_few_seconds() {
        let mut t = Traffic::default();
        let seen = blockage(false);
        let d = t.decide(0.0, me(0.0, 10.0), &seen, &TwoLane);
        assert_eq!(d.mode, Mode::Blocked);
        assert_eq!(d.attended, Some(false));
        let stop = d.stop_at.unwrap();
        assert!((stop - (seen[0].front() - BLOCKED_GAP_M - TRUCK_HALF_L)).abs() < 0.1, "stop at {stop}");
        // Still driving up to it: the wait has not started.
        assert!(t.decide(1.0, me(stop - 10.0, 4.0), &seen, &TwoLane).bypass.is_none());
        // Stopped in front of it at 2 s: five seconds later, round it.
        assert!(t.decide(2.0, me(stop, 0.0), &seen, &TwoLane).bypass.is_none());
        assert!(t.decide(6.0, me(stop, 0.0), &seen, &TwoLane).bypass.is_none(), "too early");
        let b = t.decide(7.1, me(stop, 0.0), &seen, &TwoLane).bypass.expect("bypass 5 s after stopping");
        assert_eq!(b.blockers.len(), 2);
        assert!(b.rejoin_s > seen[0].back());
        // Asked once.
        assert!(t.decide(7.5, me(stop, 0.0), &seen, &TwoLane).bypass.is_none());
        // The first detour could not be planned: the second try is on the other side.
        t.bypass_failed();
        let d = t.decide(8.0, me(stop, 0.0), &seen, &TwoLane);
        assert_eq!(d.bypass.unwrap().side, -b.side);
    }

    #[test]
    fn someone_near_the_blockage_is_honked_at_and_waited_for_a_minute() {
        let mut t = Traffic::default();
        // Nobody in the truck across the road, but a walker 12 m from it.
        let mut seen = blockage(false);
        seen.push(Seen { id: "walker".into(), kind: Kind::Person, s: 52.0, d: 4.0, half_along: 0.3, half_across: 0.3, v_along: 0.0, v_across: 0.0, occupied: true });
        let stop = t.decide(0.0, me(0.0, 10.0), &seen, &TwoLane).stop_at.unwrap();
        let mut honks = Vec::new();
        let mut bypass_at = None;
        for i in 1..=700 {
            let now = i as f64 * 0.1;
            let d = t.decide(now, me(stop, 0.0), &seen, &TwoLane);
            assert_eq!(d.attended, Some(true));
            if d.honk {
                honks.push(now);
            }
            if d.bypass.is_some() && bypass_at.is_none() {
                bypass_at = Some(now);
            }
        }
        assert_eq!(honks.len(), 2, "{honks:?}");
        assert!((honks[0] - 10.0).abs() < 0.15 && (honks[1] - 40.0).abs() < 0.15, "{honks:?}");
        let at = bypass_at.expect("bypass after a minute");
        assert!((at - 60.0).abs() < 0.15, "bypass at {at}");
        // Somebody aboard the truck across the road counts too.
        let mut t = Traffic::default();
        let seen = blockage(true);
        assert_eq!(t.decide(0.0, me(0.0, 10.0), &seen, &TwoLane).attended, Some(true));
        // Someone 40 m away does not.
        let mut t = Traffic::default();
        let mut seen = blockage(false);
        seen.push(Seen { id: "far".into(), kind: Kind::Person, s: 85.0, d: 0.0, half_along: 0.3, half_across: 0.3, v_along: 0.0, v_across: 0.0, occupied: true });
        assert_eq!(t.decide(0.0, me(0.0, 10.0), &seen, &TwoLane).attended, Some(false));
    }

    #[test]
    fn drives_on_when_the_blockage_moves_away() {
        let mut t = Traffic::default();
        let across = Seen { id: "across".into(), kind: Kind::Vehicle, s: 40.0, d: 0.0, half_along: 0.9, half_across: 3.0, v_along: 0.0, v_across: 0.0, occupied: false };
        let d = t.decide(0.0, me(20.0, 0.0), std::slice::from_ref(&across), &TwoLane);
        assert_eq!(d.mode, Mode::Blocked);
        // Gone: hold a moment, then cruise.
        let d = t.decide(20.0, me(20.0, 0.0), &[], &TwoLane);
        assert!(d.stop_at.is_some(), "still holding");
        let d = t.decide(21.5, me(20.0, 0.0), &[], &TwoLane);
        assert_eq!(d.mode, Mode::Cruise);
        assert!(d.stop_at.is_none());
        assert_eq!(t.blocked_s(21.5), 0.0);
    }

    // ── Closed loop: controller + traffic + the bicycle sim ──

    use crate::actions::driving::{
        controller::{Step, Tuning},
        testkit::{R, Sim, params, road},
    };

    fn tangent() -> geo::Tangent {
        geo::Tangent::at(geo::scale(geo::lonlat_to_unit(0.0, 0.0), R))
    }

    /// A box at (x, y) east/north, its length along `yaw` (CCW from east).
    fn boxed(id: &str, kind: Kind, x: f64, y: f64, yaw: f64, half_w: f64, half_l: f64) -> Obstacle {
        let tan = tangent();
        let axis = geo::add(geo::scale(tan.east, yaw.cos()), geo::scale(tan.north, yaw.sin()));
        Obstacle { id: id.into(), kind, pos: tan.unproject(x, y), axis, half_w, half_l, vel: Vec3::default(), occupied: false }
    }

    /// Gap between two boxes (x, y, yaw, half_w, half_l), sampled along their
    /// outlines; 0 when they touch.
    fn gap(a: (f64, f64, f64, f64, f64), b: (f64, f64, f64, f64, f64)) -> f64 {
        let outline = |(x, y, yaw, hw, hl): (f64, f64, f64, f64, f64)| {
            let (c, s) = (yaw.cos(), yaw.sin());
            let mut pts = Vec::new();
            for i in 0..=20 {
                let t = -1.0 + 2.0 * i as f64 / 20.0;
                for (u, v) in [(t * hl, hw), (t * hl, -hw), (hl, t * hw), (-hl, t * hw)] {
                    pts.push((x + u * c - v * s, y + u * s + v * c));
                }
            }
            pts
        };
        let dist = |(px, py): (f64, f64), (x, y, yaw, hw, hl): (f64, f64, f64, f64, f64)| {
            let (dx, dy) = (px - x, py - y);
            let (u, v) = (dx * yaw.cos() + dy * yaw.sin(), -dx * yaw.sin() + dy * yaw.cos());
            let (eu, ev) = ((u.abs() - hl).max(0.0), (v.abs() - hw).max(0.0));
            eu.hypot(ev)
        };
        let ab = outline(a).into_iter().map(|p| dist(p, b)).fold(f64::INFINITY, f64::min);
        let ba = outline(b).into_iter().map(|p| dist(p, a)).fold(f64::INFINITY, f64::min);
        ab.min(ba)
    }

    struct Run {
        min_gap: f64,
        modes: Vec<Mode>,
        bypasses: u32,
        stopped_at: Option<f64>,
    }

    /// Drive the sim along a two-lane road with obstacles for `secs`.
    fn drive(sim: &mut Sim, obstacles: &[Obstacle], secs: f64) -> Run {
        let tan = tangent();
        let boxes: Vec<(f64, f64, f64, f64, f64)> = obstacles
            .iter()
            .map(|o| {
                let (x, y) = tan.project(o.pos);
                let (ax, ay) = tan.project_dir(o.axis);
                (x, y, ay.atan2(ax), o.half_w, o.half_l)
            })
            .collect();
        let tuning = Tuning { max_speed_mps: 40.0 / 3.6, ..Tuning::default() };
        let mut ctrl = Controller::new(road(500.0), params(), tuning);
        let mut traffic = Traffic::default();
        let mut run = Run { min_gap: f64::INFINITY, modes: Vec::new(), bypasses: 0, stopped_at: None };
        let (dt, p) = (0.1, params());
        let mut t = 0.0;
        while t < secs {
            let obs = sim.obs();
            let seen: Vec<Seen> = obstacles.iter().filter_map(|o| seen(&ctrl, obs.pos, o, 200.0)).collect();
            let me = Me { s: ctrl.s_now(), d: ctrl.lateral_m(), v: sim.v, cruise_mps: 40.0 / 3.6 };
            let d = traffic.decide(t, me, &seen, &ctrl);
            if run.modes.last() != Some(&d.mode) {
                run.modes.push(d.mode);
            }
            ctrl.set_manoeuvre(d.manoeuvre);
            ctrl.set_speed_cap(d.speed_cap);
            ctrl.set_stop_at(d.stop_at);
            if let Some(req) = d.bypass {
                let (pts, len) = detour(&ctrl, &req, obstacles, &[], obs.pos, DETOUR_KMH).expect("a way round");
                ctrl = Controller::new(pts, params(), detour_tuning(tuning));
                traffic.new_route(Some(len));
                run.bypasses += 1;
            }
            if d.mode == Mode::Blocked && sim.v.abs() < 0.05 {
                run.stopped_at = Some(sim.x);
            }
            match ctrl.step(&sim.obs(), dt) {
                Step::Drive(c) => {
                    for _ in 0..6 {
                        sim.step(&c, &p, dt / 6.0);
                    }
                }
                Step::Arrived => break,
                Step::NeedsReset => panic!("stuck at ({:.1}, {:.1})", sim.x, sim.y),
            }
            let me_box = (sim.x, sim.y, sim.yaw, TRUCK_HALF_W, TRUCK_HALF_L);
            for b in &boxes {
                run.min_gap = run.min_gap.min(gap(me_box, *b));
            }
            t += dt;
        }
        run
    }

    #[test]
    fn overtakes_the_parked_truck_of_the_screenshot() {
        let parked = boxed("parked", Kind::Vehicle, 120.0, -1.75, 0.0, TRUCK_HALF_W, TRUCK_HALF_L);
        let mut sim = Sim { x: 0.0, y: -1.75, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        let run = drive(&mut sim, &[parked], 45.0);
        assert!(run.modes.contains(&Mode::Avoid), "{:?}", run.modes);
        assert!(run.min_gap > 0.3, "came within {:.2} m", run.min_gap);
        assert!(sim.x > 250.0, "only reached {:.1}", sim.x);
        assert!((sim.y + 1.75).abs() < 0.5, "not back in the right lane: {:.2}", sim.y);
    }

    #[test]
    fn stops_at_a_blocked_road_then_drives_round_it() {
        // A truck parked across the road and a crate filling the rest; nobody
        // about, so after a few seconds the truck goes round off the road.
        let across = boxed("across", Kind::Vehicle, 150.0, -0.5, std::f64::consts::FRAC_PI_2, TRUCK_HALF_W, TRUCK_HALF_L);
        let crate_ = boxed("crate", Kind::Prop, 150.5, 2.4, 0.0, 0.65, 1.25);
        let mut sim = Sim { x: 0.0, y: -1.75, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        let run = drive(&mut sim, &[across, crate_], 90.0);
        assert!(run.modes.contains(&Mode::Blocked), "{:?}", run.modes);
        let stop = run.stopped_at.expect("stopped");
        assert!((135.0..=139.5).contains(&stop), "stopped at {stop:.1}");
        assert_eq!(run.bypasses, 1, "{:?}", run.modes);
        assert!(run.min_gap > 0.3, "came within {:.2} m", run.min_gap);
        assert!(sim.x > 220.0, "only reached {:.1}", sim.x);
    }

    #[test]
    fn a_detour_keeps_clear_of_what_stands_beside_the_road_too() {
        // The blockage, plus containers on the left verge: the detour must go
        // right, or round them as well, without touching any.
        let across = boxed("across", Kind::Vehicle, 150.0, -0.5, std::f64::consts::FRAC_PI_2, TRUCK_HALF_W, TRUCK_HALF_L);
        let crate_ = boxed("crate", Kind::Prop, 150.5, 2.4, 0.0, 0.65, 1.25);
        let verge = boxed("verge", Kind::Prop, 150.0, 7.0, 0.0, 1.2, 6.0);
        let mut sim = Sim { x: 0.0, y: -1.75, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        let run = drive(&mut sim, &[across, crate_, verge], 90.0);
        assert_eq!(run.bypasses, 1, "{:?}", run.modes);
        assert!(run.min_gap > 0.3, "came within {:.2} m", run.min_gap);
        assert!(sim.x > 220.0, "only reached {:.1}", sim.x);
    }

    #[test]
    fn free_offset_picks_the_nearest_gap() {
        let o = |d: f64, h: f64| Seen { id: "x".into(), kind: Kind::Prop, s: 0.0, d, half_along: 1.0, half_across: h, v_along: 0.0, v_across: 0.0, occupied: false };
        let (a, b) = (o(-1.5, 0.9), o(3.4, 0.3));
        let range = lanes::lanes(3.0, 2).centre_range(TRUCK_HALF_W, EDGE_M, OFFROAD_HALF_M);
        // Left of the first, right of the second: room between them.
        let got = free_offset(range, &[&a, &b], -1.5).unwrap();
        let reach = TRUCK_HALF_W + CLEARANCE_M;
        assert!(got >= -1.5 + 0.9 + reach - 1e-9 && got <= 3.4 - 0.3 - reach + 1e-9, "{got}");
        let wall = o(0.0, 3.0);
        assert!(free_offset(range, &[&wall], -1.5).is_none());
    }
}
