//! Route-following controller for an NPC-driven ground vehicle. Pure maths,
//! no I/O: fed an [`Observation`] of the vehicle (as replicated by Horizon) it
//! answers the pilot inputs to send (`vehicle_input {throttle, steer, brake}`).
//!
//! It mirrors what the game expects from a human pilot (`Vehicle._apply_drive`,
//! `VehicleSteering.step`):
//!
//! - **steer is a RATE**, not an angle: ±1 turns the front wheels at
//!   `turn_speed` rad/s and they stay where they are left. So the controller
//!   computes the wheel angle it wants (pure pursuit) and drives the replicated
//!   `steering` angle toward it with a proportional command. Positive = left.
//! - **throttle** in [-1, 1]: forward drive; against the motion it is the
//!   service brake, and only from (nearly) standstill does it reverse.
//! - **brake**: the wheel brakes.
//!
//! Geometry is done in the tangent plane at the vehicle (east/north metres):
//! altitude is left to the physics, which keeps the wheels on the terrain.
//!
//! The vehicle does not follow the road's centreline but a lateral OFFSET from
//! it (positive = left): the lane it cruises in ([`lanes`]), blended with a
//! [`Manoeuvre`] the traffic logic sets (overtake, avoid, squeeze right). The
//! traffic logic can also cap the speed and ask for a stop at a point of the
//! route ([`Controller::set_speed_cap`], [`Controller::set_stop_at`]).
//!
//! [`lanes`]: super::lanes

use super::lanes::{self, Lanes};
use crate::{horizon::messages::Vec3, world_data::geo};

/// One point of the route to follow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePt {
    /// Planet-frame unit direction.
    pub unit: Vec3,
    /// Cumulative metres along the route.
    pub s: f64,
    pub speed_kmh: f64,
    /// The road there (0 and 0 off the road).
    pub half_width_m: f64,
    pub lanes: u8,
}

impl From<(Vec3, f64, f64)> for RoutePt {
    /// An off-road point (the vehicle follows the line itself).
    fn from((unit, s, speed_kmh): (Vec3, f64, f64)) -> Self {
        Self { unit, s, speed_kmh, half_width_m: 0.0, lanes: 0 }
    }
}

/// A lateral move the traffic logic asks for: from `s_in0` the offset blends
/// from the lane's toward `d_m`, reaching it at `s_in1`, holds it until
/// `s_out0`, and blends back to the lane by `s_out1` (smoothstep both ways).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Manoeuvre {
    pub s_in0: f64,
    pub s_in1: f64,
    pub s_out0: f64,
    pub s_out1: f64,
    pub d_m: f64,
}

impl Manoeuvre {
    /// Weight of the manoeuvre's offset at `s` (0 = the lane, 1 = `d_m`).
    pub fn weight(&self, s: f64) -> f64 {
        let ramp = |a: f64, b: f64, x: f64| {
            if b <= a {
                return if x >= a { 1.0 } else { 0.0 };
            }
            let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        ramp(self.s_in0, self.s_in1, s) * (1.0 - ramp(self.s_out0, self.s_out1, s))
    }
}

/// Where a point lies relative to the route: `s` along it, `d` across
/// (positive = left of the direction of travel), and the route's direction
/// there in the tangent plane the query was made in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteSpot {
    pub s: f64,
    pub d: f64,
    pub dir: (f64, f64),
}

/// Lane offsets are smoothed over this distance either side, so the vehicle
/// eases into its lane where the route leaves or joins a road.
const LANE_BLEND_M: f64 = 10.0;

/// Pilot inputs, exactly as sent in `vehicle_input`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Command {
    pub throttle: f64,
    pub steer: f64,
    pub brake: bool,
}

impl Command {
    pub const STOP: Command = Command { throttle: 0.0, steer: 0.0, brake: true };

    /// Whether `other` differs enough from `self` to be worth sending.
    pub fn differs(&self, other: &Command) -> bool {
        self.brake != other.brake
            || (self.throttle - other.throttle).abs() > 0.04
            || (self.steer - other.steer).abs() > 0.04
    }
}

/// What the vehicle looks like right now.
#[derive(Debug, Clone, Copy)]
pub struct Observation {
    /// Position, planet frame (metres from the planet centre).
    pub pos: Vec3,
    /// The vehicle's forward direction, planet frame (need not be horizontal).
    pub forward: Vec3,
    /// Signed forward speed, m/s (negative = rolling backwards).
    pub speed_mps: f64,
    /// Current front-wheel angle (rad, positive = left), when replicated.
    pub steering_rad: Option<f64>,
    /// Seconds since this observation was replicated.
    pub age_s: f64,
}

/// Steering geometry and response of the vehicle (from its scene layout).
#[derive(Debug, Clone, Copy)]
pub struct VehicleParams {
    pub wheelbase_m: f64,
    pub max_steer_rad: f64,
    pub steer_min_ratio: f64,
    pub steer_falloff_kmh: f64,
    /// Wheel turn rate at steer = ±1, turning OUT from centre
    /// (VehicleSteering.turn_speed; the truck's is a slow 0.2 rad/s).
    pub turn_speed_rad_s: f64,
    /// Wheel turn rate back TOWARD centre (VehicleSteering.return_speed).
    pub return_speed_rad_s: f64,
}

/// |steer| below this does nothing (VehicleSteering.DEAD_ZONE): the wheels
/// drift back toward centre instead.
const STEER_DEAD_ZONE: f64 = 0.01;

impl VehicleParams {
    /// Wheel turn rate for a steer command of sign `steer` with the wheels at
    /// `now`: back toward centre is faster than out.
    pub fn rate(&self, steer: f64, now: f64) -> f64 {
        if now != 0.0 && steer.signum() != now.signum() { self.return_speed_rad_s } else { self.turn_speed_rad_s }
    }
}

impl VehicleParams {
    /// Largest wheel angle at `speed_mps` (VehicleSteering.lock_rad).
    pub fn lock_rad(&self, speed_mps: f64) -> f64 {
        if self.steer_falloff_kmh <= 0.0 {
            return self.max_steer_rad;
        }
        let t = (speed_mps.abs() * 3.6 / self.steer_falloff_kmh).clamp(0.0, 1.0);
        self.max_steer_rad * (1.0 + (self.steer_min_ratio - 1.0) * t)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// Hard speed cap, m/s.
    pub max_speed_mps: f64,
    /// Lookahead = clamp(k · speed, min, max).
    pub lookahead_k_s: f64,
    pub lookahead_min_m: f64,
    pub lookahead_max_m: f64,
    /// Lateral acceleration allowed in turns, m/s² (sets the corner speed).
    pub lat_accel_mps2: f64,
    /// Planned deceleration toward slower stretches and the final stop, m/s².
    pub decel_mps2: f64,
    /// Speed loop gains (throttle per m/s of error, and per m of integrated error).
    pub kp: f64,
    pub ki: f64,
    /// Time constant of the wheel-angle loop, s.
    pub steer_tau_s: f64,
    /// Done when this close to the end of the route and (nearly) stopped.
    pub arrive_m: f64,
    /// Beyond this off the route, re-find the closest point anywhere on it.
    pub lost_m: f64,
    /// Observations older than this stop the vehicle.
    pub stale_s: f64,
    /// Pushing but not moving for this long = stuck.
    pub stuck_s: f64,
    /// How long one unstick manoeuvre (reverse, opposite lock) lasts.
    pub unstick_s: f64,
    /// Unstick attempts before asking for a vehicle reset.
    pub unstick_attempts: u32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            max_speed_mps: 50.0 / 3.6,
            lookahead_k_s: 1.2,
            lookahead_min_m: 7.0,
            lookahead_max_m: 25.0,
            lat_accel_mps2: 2.0,
            decel_mps2: 1.5,
            kp: 0.35,
            ki: 0.04,
            steer_tau_s: 0.35,
            arrive_m: 4.0,
            lost_m: 25.0,
            stale_s: 1.0,
            stuck_s: 8.0,
            unstick_s: 2.5,
            unstick_attempts: 3,
        }
    }
}

/// What the controller wants this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Drive(Command),
    /// At the end of the route and stopped: hold the brakes, the trip is done.
    Arrived,
    /// Stuck after every unstick attempt: the caller should `reset_vehicle`
    /// and may then carry on.
    NeedsReset,
}

#[derive(Debug, Clone, Copy)]
struct Pt {
    unit: Vec3,
    s: f64,
    speed_mps: f64,
    lanes: Lanes,
    /// Lane offset there, smoothed along the route.
    base_d: f64,
    src: RoutePt,
}

#[derive(Clone)]
pub struct Controller {
    pts: Vec<Pt>,
    length_m: f64,
    params: VehicleParams,
    tuning: Tuning,
    /// Index of the route point closest to the vehicle (only moves forward,
    /// except when re-found after being lost).
    idx: usize,
    integ: f64,
    /// Our own estimate of the wheel angle, when it is not replicated.
    steer_est: f64,
    stuck_for: f64,
    unstick_left: f64,
    unstick_dir: f64,
    unsticks: u32,
    manoeuvre: Option<Manoeuvre>,
    speed_cap: Option<f64>,
    stop_at: Option<f64>,
    /// Where the vehicle was at the last step: along the route, and across it.
    s_now: f64,
    lateral: f64,
    /// Distance off the path being followed (lane + manoeuvre) at the last
    /// step, for logging.
    pub cross_track_m: f64,
    /// Target speed at the last step, for logging.
    pub target_speed_mps: f64,
}

impl Controller {
    /// `route`: the points to follow ([`RoutePt`], or `(unit, s, km/h)`
    /// tuples for an off-road line).
    pub fn new(route: impl IntoIterator<Item = impl Into<RoutePt>>, params: VehicleParams, tuning: Tuning) -> Self {
        let mut pts: Vec<Pt> = route
            .into_iter()
            .map(|r| {
                let r: RoutePt = r.into();
                let l = lanes::lanes(r.half_width_m, r.lanes);
                Pt { unit: r.unit, s: r.s, speed_mps: r.speed_kmh / 3.6, lanes: l, base_d: l.cruise_m, src: r }
            })
            .collect();
        // Smooth the lane offset over ±LANE_BLEND_M (two pointers over s).
        let raw: Vec<f64> = pts.iter().map(|p| p.lanes.cruise_m).collect();
        let (mut lo, mut hi, mut sum) = (0usize, 0usize, 0.0);
        for i in 0..pts.len() {
            while hi < pts.len() && pts[hi].s <= pts[i].s + LANE_BLEND_M {
                sum += raw[hi];
                hi += 1;
            }
            while pts[lo].s < pts[i].s - LANE_BLEND_M {
                sum -= raw[lo];
                lo += 1;
            }
            pts[i].base_d = sum / (hi - lo) as f64;
        }
        let length_m = pts.last().map(|p| p.s).unwrap_or(0.0);
        Self {
            pts,
            length_m,
            params,
            tuning,
            idx: 0,
            integ: 0.0,
            steer_est: 0.0,
            stuck_for: 0.0,
            unstick_left: 0.0,
            unstick_dir: 0.0,
            unsticks: 0,
            manoeuvre: None,
            speed_cap: None,
            stop_at: None,
            s_now: 0.0,
            lateral: 0.0,
            cross_track_m: 0.0,
            target_speed_mps: 0.0,
        }
    }

    // ── What the traffic logic sets ──

    /// The lateral move to blend in (`None` = keep to the lane).
    pub fn set_manoeuvre(&mut self, m: Option<Manoeuvre>) {
        self.manoeuvre = m;
    }

    pub fn manoeuvre(&self) -> Option<Manoeuvre> {
        self.manoeuvre
    }

    /// Speed not to exceed, m/s (`None` = the road's).
    pub fn set_speed_cap(&mut self, cap: Option<f64>) {
        self.speed_cap = cap;
    }

    /// Stop at this distance along the route (`None` = only at its end).
    pub fn set_stop_at(&mut self, s: Option<f64>) {
        self.stop_at = s;
    }

    // ── What it reads ──

    /// Where the vehicle was along the route at the last step.
    pub fn s_now(&self) -> f64 {
        self.s_now
    }

    /// How far left (+) or right (−) of the centreline it was at the last step.
    pub fn lateral_m(&self) -> f64 {
        self.lateral
    }

    /// The speed limit at `s` (capped by the tuning), m/s.
    pub fn speed_limit_at(&self, s: f64) -> f64 {
        self.pts.get(self.seg_at(s)).map(|p| p.speed_mps).unwrap_or(0.0).min(self.tuning.max_speed_mps)
    }

    /// The lanes of the road at `s`.
    pub fn lanes_at(&self, s: f64) -> Lanes {
        self.pts.get(self.seg_at(s)).map(|p| p.lanes).unwrap_or_else(Lanes::offroad)
    }

    /// The lane offset at `s`, without any manoeuvre.
    pub fn base_offset_at(&self, s: f64) -> f64 {
        let i = self.seg_at(s);
        match (self.pts.get(i), self.pts.get(i + 1)) {
            (Some(a), Some(b)) if b.s > a.s => {
                let t = ((s - a.s) / (b.s - a.s)).clamp(0.0, 1.0);
                a.base_d + (b.base_d - a.base_d) * t
            }
            (Some(a), _) => a.base_d,
            _ => 0.0,
        }
    }

    /// The offset the vehicle aims for at `s`: its lane, blended with the
    /// manoeuvre in force.
    pub fn offset_at(&self, s: f64) -> f64 {
        let base = self.base_offset_at(s);
        match self.manoeuvre {
            Some(m) => base + (m.d_m - base) * m.weight(s),
            None => base,
        }
    }

    /// Where planet-frame point `p` lies relative to the route, measured in
    /// the tangent plane at `vehicle` — `None` when it is more than `max_d_m`
    /// off the stretch from 50 m behind to `ahead_m` ahead of the vehicle.
    pub fn project(&self, vehicle: Vec3, p: Vec3, ahead_m: f64, max_d_m: f64) -> Option<RouteSpot> {
        if self.pts.len() < 2 {
            return None;
        }
        let radius = geo::length(vehicle);
        let tan = geo::Tangent::at(vehicle);
        let at = |q: &Pt| tan.project(geo::scale(q.unit, radius));
        let (px, py) = tan.project(geo::scale(geo::normalized(p), radius));
        let first = self.seg_at(self.s_now - 50.0);
        let mut best: Option<(f64, RouteSpot)> = None;
        for i in first..self.pts.len() - 1 {
            let (a, b) = (&self.pts[i], &self.pts[i + 1]);
            if a.s > self.s_now + ahead_m {
                break;
            }
            let ((ax, ay), (bx, by)) = (at(a), at(b));
            let (dx, dy) = (bx - ax, by - ay);
            let len = dx.hypot(dy);
            if len < 1e-6 {
                continue;
            }
            let (ux, uy) = (dx / len, dy / len);
            let t = (((px - ax) * ux + (py - ay) * uy) / len).clamp(0.0, 1.0);
            let (cx, cy) = (ax + dx * t, ay + dy * t);
            let dist = (px - cx).hypot(py - cy);
            if best.as_ref().is_none_or(|(d, _)| dist < *d) {
                let d = ux * (py - cy) - uy * (px - cx);
                best = Some((dist, RouteSpot { s: a.s + (b.s - a.s) * t, d, dir: (ux, uy) }));
            }
        }
        best.filter(|(dist, _)| *dist <= max_d_m).map(|(_, spot)| spot)
    }

    /// Planet-frame point at `s` along the route, `d` metres to its left.
    pub fn position_at(&self, s: f64, d: f64, radius: f64) -> Vec3 {
        let i = self.seg_at(s).min(self.pts.len().saturating_sub(2));
        let (a, b) = (&self.pts[i], &self.pts[(i + 1).min(self.pts.len() - 1)]);
        let t = if b.s > a.s { ((s - a.s) / (b.s - a.s)).clamp(0.0, 1.0) } else { 0.0 };
        let centre = geo::scale(geo::normalized(geo::add(geo::scale(a.unit, 1.0 - t), geo::scale(b.unit, t))), radius);
        let tan = geo::Tangent::at(centre);
        let (bx, by) = tan.project(geo::scale(b.unit, radius));
        let (ax, ay) = tan.project(geo::scale(a.unit, radius));
        let (dx, dy) = (bx - ax, by - ay);
        let n = dx.hypot(dy).max(1e-9);
        tan.unproject(-dy / n * d, dx / n * d)
    }

    /// The route from `s` on (for re-planning around an obstacle), with `s`
    /// counted from the start of the route.
    pub fn points_from(&self, s: f64) -> Vec<RoutePt> {
        self.pts.iter().filter(|p| p.s >= s).map(|p| p.src).collect()
    }

    /// Index of the segment containing `s` (the last point past the end).
    fn seg_at(&self, s: f64) -> usize {
        match self.pts.binary_search_by(|p| p.s.total_cmp(&s)) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        }
    }

    pub fn length_m(&self) -> f64 {
        self.length_m
    }

    /// Distance along the route of the point closest to the vehicle.
    pub fn progress_m(&self) -> f64 {
        self.pts.get(self.idx).map(|p| p.s).unwrap_or(0.0)
    }

    pub fn remaining_m(&self) -> f64 {
        self.length_m - self.progress_m()
    }

    /// Forget the stuck bookkeeping (after a vehicle reset).
    pub fn reset_stuck(&mut self) {
        self.stuck_for = 0.0;
        self.unstick_left = 0.0;
        self.unsticks = 0;
        self.integ = 0.0;
    }

    /// One control step, `dt` seconds after the previous one.
    pub fn step(&mut self, obs: &Observation, dt: f64) -> Step {
        if self.pts.len() < 2 {
            return Step::Arrived;
        }
        if obs.age_s > self.tuning.stale_s {
            self.integ = 0.0;
            return Step::Drive(Command { throttle: 0.0, steer: 0.0, brake: true });
        }
        let radius = geo::length(obs.pos);
        let tan = geo::Tangent::at(obs.pos);
        let at = |p: &Pt| tan.project(geo::scale(p.unit, radius));

        // ── Where are we along the route ──
        self.advance(&at);
        let (cx, cy) = at(&self.pts[self.idx]);
        if cx.hypot(cy) > self.tuning.lost_m {
            self.refind(&at);
        }
        self.locate(&at);
        self.cross_track_m = (self.lateral - self.offset_at(self.s_now)).abs();
        let speed = obs.speed_mps;
        let remaining = self.remaining_m();
        let end = at(self.pts.last().unwrap());
        let end_dist = end.0.hypot(end.1);

        if remaining <= self.tuning.arrive_m || end_dist <= self.tuning.arrive_m {
            // In the arrival zone (or just past it): stop dead, wheels as they
            // are — chasing a point now behind would only start circling.
            if speed.abs() < 0.5 {
                return Step::Arrived;
            }
            self.integ = 0.0;
            return Step::Drive(Command { throttle: 0.0, steer: 0.0, brake: true });
        }

        // ── Unstick manoeuvre in progress ──
        let steering_now = obs.steering_rad.unwrap_or(self.steer_est);
        if self.unstick_left > 0.0 {
            self.unstick_left -= dt;
            let lock = self.params.lock_rad(speed);
            let cmd = Command {
                throttle: -0.6,
                steer: self.steer_toward(self.unstick_dir * lock, steering_now),
                brake: false,
            };
            self.track_steer(cmd.steer, dt, speed);
            return Step::Drive(cmd);
        }

        // ── Steering: pure pursuit ──
        let heading = {
            let (hx, hy) = tan.project_dir(obs.forward);
            let n = hx.hypot(hy);
            if n < 1e-6 { (0.0, 1.0) } else { (hx / n, hy / n) }
        };
        let ld = (self.tuning.lookahead_k_s * speed.abs())
            .clamp(self.tuning.lookahead_min_m, self.tuning.lookahead_max_m);
        let target = self.lookahead_point(&at, ld);
        let (tx, ty) = target;
        let dist_t = tx.hypot(ty).max(0.1);
        // Signed angle from heading to target, CCW (= left) positive.
        let alpha = (heading.0 * ty - heading.1 * tx).atan2(heading.0 * tx + heading.1 * ty);
        let lock = self.params.lock_rad(speed);
        let want = if alpha.abs() > std::f64::consts::FRAC_PI_2 {
            // Target behind: pure pursuit's sin α fades to nothing there, so
            // just turn hard toward it (left when dead behind).
            if alpha >= 0.0 { lock } else { -lock }
        } else {
            (2.0 * self.params.wheelbase_m * alpha.sin() / dist_t)
                .atan()
                .clamp(-lock, lock)
        };
        let steer = self.steer_toward(want, steering_now);

        // ── Speed ──
        let mut v_target = self.target_speed(&at, speed);
        // Wheels far from the angle wanted (a slow-steering truck entering a
        // bend or a lane change): slow down so they get there within a
        // fraction of the lookahead, instead of cutting the corner.
        let lag = (want - steering_now).abs();
        if lag > 0.02 {
            let catch_up_s = lag / self.params.rate(want - steering_now, steering_now).max(1e-3);
            v_target = v_target.min(((0.4 * ld) / catch_up_s).max(1.5));
        }
        // Facing away from the target (e.g. just spawned the wrong way round):
        // crawl while the wheels bring the nose round.
        if alpha.abs() > std::f64::consts::FRAC_PI_2 {
            v_target = v_target.min(2.0);
        }
        self.target_speed_mps = v_target;
        let err = v_target - speed;
        self.integ = (self.integ + err * dt).clamp(-20.0, 20.0);
        let mut throttle = self.tuning.kp * err + self.tuning.ki * self.integ;
        let mut brake = false;
        if err < -1.0 {
            // Too fast: service brake through a negative throttle, harder when
            // far over; the wheel brakes on top when well over.
            throttle = (self.tuning.kp * err).clamp(-1.0, 0.0);
            brake = err < -3.0;
            self.integ = 0.0;
        } else {
            throttle = throttle.clamp(0.0, 1.0);
        }
        // Asked to stand still and nearly there: brake rather than coast the
        // last metres (a speed error under 1 m/s does not reach the brakes).
        if v_target < 0.3 && speed.abs() < 2.0 {
            throttle = 0.0;
            brake = true;
        }

        // ── Stuck? ──
        if throttle > 0.3 && speed.abs() < 0.5 {
            self.stuck_for += dt;
        } else {
            self.stuck_for = 0.0;
        }
        if self.stuck_for >= self.tuning.stuck_s {
            self.stuck_for = 0.0;
            self.integ = 0.0;
            if self.unsticks >= self.tuning.unstick_attempts {
                self.unsticks = 0;
                return Step::NeedsReset;
            }
            self.unsticks += 1;
            self.unstick_left = self.tuning.unstick_s;
            // Back out with the wheels turned the other way, so the nose
            // swings toward the side the route lies on.
            self.unstick_dir = if alpha >= 0.0 { -1.0 } else { 1.0 };
        }

        self.track_steer(steer, dt, speed);
        Step::Drive(Command { throttle, steer, brake })
    }

    /// Proportional steer-rate command bringing the wheels from `now` to
    /// `want` (radians) in about `steer_tau_s`, at the rate the wheels turn in
    /// that direction. Kept out of the game's dead zone while there is
    /// something to correct.
    fn steer_toward(&self, want: f64, now: f64) -> f64 {
        let err = want - now;
        let cmd = (err / (self.params.rate(err, now) * self.tuning.steer_tau_s)).clamp(-1.0, 1.0);
        if err.abs() > 0.002 && cmd.abs() <= STEER_DEAD_ZONE { STEER_DEAD_ZONE * 1.5 * cmd.signum() } else { cmd }
    }

    /// Integrate our own wheel-angle estimate (used only when the vehicle does
    /// not replicate `steering`).
    fn track_steer(&mut self, steer: f64, dt: f64, speed: f64) {
        let lock = self.params.lock_rad(speed);
        self.steer_est =
            (self.steer_est + steer * self.params.rate(steer, self.steer_est) * dt).clamp(-lock, lock);
    }

    /// `s_now` and `lateral` from the vehicle's projection (the origin of the
    /// tangent plane) onto the segments either side of `idx`.
    fn locate(&mut self, at: &impl Fn(&Pt) -> (f64, f64)) {
        let lo = self.idx.saturating_sub(1);
        let hi = (self.idx + 1).min(self.pts.len() - 1);
        let mut best = (f64::INFINITY, self.pts[self.idx].s, 0.0);
        for i in lo..hi {
            let (a, b) = (&self.pts[i], &self.pts[i + 1]);
            let ((ax, ay), (bx, by)) = (at(a), at(b));
            let (dx, dy) = (bx - ax, by - ay);
            let len = dx.hypot(dy);
            if len < 1e-6 {
                continue;
            }
            let (ux, uy) = (dx / len, dy / len);
            let t = ((-ax * ux - ay * uy) / len).clamp(0.0, 1.0);
            let (qx, qy) = (ax + dx * t, ay + dy * t);
            let dist = qx.hypot(qy);
            if dist < best.0 {
                // The vehicle is at the origin: (0,0) − (qx,qy).
                best = (dist, a.s + (b.s - a.s) * t, ux * (-qy) - uy * (-qx));
            }
        }
        self.s_now = best.1;
        self.lateral = best.2;
    }

    /// Move `idx` forward to the closest point within a window ahead.
    fn advance(&mut self, at: &impl Fn(&Pt) -> (f64, f64)) {
        const WINDOW: usize = 60;
        let end = (self.idx + WINDOW).min(self.pts.len() - 1);
        let mut best = (self.idx, f64::INFINITY);
        for i in self.idx..=end {
            let (x, y) = at(&self.pts[i]);
            let d = x.hypot(y);
            if d < best.1 {
                best = (i, d);
            }
        }
        self.idx = best.0;
    }

    /// Closest point anywhere on the route.
    fn refind(&mut self, at: &impl Fn(&Pt) -> (f64, f64)) {
        let mut best = (self.idx, f64::INFINITY);
        for (i, p) in self.pts.iter().enumerate() {
            let (x, y) = at(p);
            let d = x.hypot(y);
            if d < best.1 {
                best = (i, d);
            }
        }
        self.idx = best.0;
    }

    /// Point `ld` metres further along the route than the vehicle (the last
    /// point when the route ends sooner), shifted sideways by the offset aimed
    /// for there; tangent-plane coordinates.
    fn lookahead_point(&self, at: &impl Fn(&Pt) -> (f64, f64), ld: f64) -> (f64, f64) {
        let s_goal = (self.s_now + ld).min(self.length_m);
        let i = self.seg_at(s_goal).min(self.pts.len() - 2);
        let (a, b) = (&self.pts[i], &self.pts[i + 1]);
        let t = ((s_goal - a.s) / (b.s - a.s).max(1e-9)).clamp(0.0, 1.0);
        let ((ax, ay), (bx, by)) = (at(a), at(b));
        let (dx, dy) = (bx - ax, by - ay);
        let n = dx.hypot(dy).max(1e-9);
        let d = self.offset_at(s_goal);
        (ax + dx * t - dy / n * d, ay + dy * t + dx / n * d)
    }

    /// Speed to aim for now: the road's limit and the cap, slowed for the
    /// tightest bend ahead and for every slower stretch (and the final stop)
    /// within braking distance.
    fn target_speed(&self, at: &impl Fn(&Pt) -> (f64, f64), speed: f64) -> f64 {
        let t = &self.tuning;
        let s0 = self.pts[self.idx].s;
        let horizon = (speed * speed / (2.0 * t.decel_mps2)).max(40.0) + 20.0;
        let mut v = self.pts[self.idx].speed_mps.min(t.max_speed_mps);
        // Final stop.
        let to_end = (self.length_m - s0).max(0.0);
        v = v.min((2.0 * t.decel_mps2 * (to_end - t.arrive_m * 0.5).max(0.0)).sqrt());
        // What the traffic logic asks for: a cap, and a stop short of an obstacle.
        if let Some(cap) = self.speed_cap {
            v = v.min(cap.max(0.0));
        }
        if let Some(stop) = self.stop_at {
            v = v.min((2.0 * t.decel_mps2 * (stop - self.s_now - 1.0).max(0.0)).sqrt());
        }
        // Curvature from three points spaced ~5 m apart, scanned ahead.
        let mut i = self.idx;
        let step = |from: usize, metres: f64| -> usize {
            let goal = self.pts[from].s + metres;
            let mut j = from;
            while j + 1 < self.pts.len() && self.pts[j].s < goal {
                j += 1;
            }
            j
        };
        while i < self.pts.len() && self.pts[i].s - s0 <= horizon {
            let p = &self.pts[i];
            let ahead = p.s - s0;
            let v_allowed_here = |v_there: f64| (v_there * v_there + 2.0 * t.decel_mps2 * ahead).sqrt();
            v = v.min(v_allowed_here(p.speed_mps.min(t.max_speed_mps)));
            let (j, k) = (step(i, 6.0), step(step(i, 6.0), 6.0));
            if k > j && j > i {
                let kappa = curvature(at(&self.pts[i]), at(&self.pts[j]), at(&self.pts[k]));
                if kappa > 1e-4 {
                    v = v.min(v_allowed_here((t.lat_accel_mps2 / kappa).sqrt()));
                }
            }
            let next = step(i, 3.0);
            if next == i {
                break;
            }
            i = next;
        }
        v.max(0.0)
    }
}

/// Curvature (1/m) of the circle through three points.
fn curvature(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    let ab = (b.0 - a.0).hypot(b.1 - a.1);
    let bc = (c.0 - b.0).hypot(c.1 - b.1);
    let ca = (a.0 - c.0).hypot(a.1 - c.1);
    let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let denom = ab * bc * ca;
    if denom < 1e-9 { 0.0 } else { 2.0 * cross.abs() / denom }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::actions::driving::testkit::{R, Sim, params, road, route, trace};

    /// Drive the sim through the controller; returns (arrived, max cross-track,
    /// sim, resets requested).
    fn run(ctrl: &mut Controller, sim: &mut Sim, secs: f64) -> (bool, f64, u32) {
        let dt = 0.1;
        let p = params();
        let mut max_ct: f64 = 0.0;
        let mut resets = 0;
        let mut t = 0.0;
        while t < secs {
            match ctrl.step(&sim.obs(), dt) {
                Step::Arrived => return (true, max_ct, resets),
                Step::NeedsReset => {
                    resets += 1;
                    ctrl.reset_stuck();
                }
                Step::Drive(c) => {
                    // The game integrates at 60 Hz between our 10 Hz commands.
                    for _ in 0..6 {
                        sim.step(&c, &p, dt / 6.0);
                    }
                }
            }
            if ctrl.progress_m() > 10.0 {
                max_ct = max_ct.max(ctrl.cross_track_m);
            }
            t += dt;
        }
        (false, max_ct, resets)
    }

    #[test]
    fn follows_an_s_bend_and_stops_at_the_end() {
        let pts = [(0.0, 0.0), (150.0, 0.0), (190.0, 30.0), (200.0, 80.0), (240.0, 120.0), (400.0, 120.0)];
        let mut ctrl = Controller::new(route(&pts, 50.0), params(), Tuning::default());
        let mut sim = Sim { x: 0.0, y: 0.0, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        let (arrived, max_ct, _) = run(&mut ctrl, &mut sim, 240.0);
        assert!(arrived, "did not arrive; at ({:.1}, {:.1}) v {:.1}", sim.x, sim.y, sim.v);
        assert!(max_ct < 3.0, "strayed {max_ct:.2} m off the route");
        assert!((sim.x - 400.0).hypot(sim.y - 120.0) < 6.0, "stopped at ({:.1}, {:.1})", sim.x, sim.y);
    }

    #[test]
    fn turns_round_when_starting_backwards() {
        // Spawned facing west, the route goes east.
        let pts = [(0.0, 0.0), (200.0, 0.0)];
        let mut ctrl = Controller::new(route(&pts, 30.0), params(), Tuning::default());
        let mut sim = Sim { x: 0.0, y: 0.0, yaw: std::f64::consts::PI, v: 0.0, steer: 0.0, blocked: false };
        let (arrived, _, _) = run(&mut ctrl, &mut sim, 240.0);
        assert!(arrived, "did not arrive; at ({:.1}, {:.1})", sim.x, sim.y);
    }

    #[test]
    fn a_blocked_vehicle_tries_to_unstick_then_asks_for_a_reset() {
        let pts = [(0.0, 0.0), (200.0, 0.0)];
        let mut ctrl = Controller::new(route(&pts, 30.0), params(), Tuning::default());
        let mut sim = Sim { x: 0.0, y: 0.0, yaw: 0.0, v: 0.0, steer: 0.0, blocked: true };
        let (_, _, resets) = run(&mut ctrl, &mut sim, 60.0);
        assert!(resets >= 1, "never asked for a reset");
    }

    #[test]
    fn stale_observations_stop_the_vehicle() {
        let mut ctrl = Controller::new(route(&[(0.0, 0.0), (100.0, 0.0)], 50.0), params(), Tuning::default());
        let sim = Sim { x: 0.0, y: 0.0, yaw: 0.0, v: 5.0, steer: 0.0, blocked: false };
        let mut o = sim.obs();
        o.age_s = 3.0;
        assert_eq!(ctrl.step(&o, 0.1), Step::Drive(Command::STOP));
    }

    #[test]
    fn keeps_to_the_right_lane() {
        let mut ctrl = Controller::new(road(400.0), params(), Tuning::default());
        let mut sim = Sim { x: 0.0, y: 0.0, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        let mut worst: f64 = 0.0;
        trace(&mut ctrl, &mut sim, 40.0, |_, s| {
            if s.x > 60.0 && s.x < 380.0 {
                worst = worst.max((s.y + 1.75).abs());
            }
        });
        assert!(worst < 0.4, "{worst:.2} m off the right lane (1.25 m from the edge)");
        assert!(ctrl.lateral_m() < -1.0, "lateral {:.2}", ctrl.lateral_m());
    }

    #[test]
    fn changes_lane_and_back_without_overshooting() {
        let mut ctrl = Controller::new(road(500.0), params(), Tuning::default());
        ctrl.set_manoeuvre(Some(Manoeuvre { s_in0: 120.0, s_in1: 150.0, s_out0: 220.0, s_out1: 250.0, d_m: 1.75 }));
        let mut sim = Sim { x: 0.0, y: -1.75, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        let (mut at_hold, mut after, mut over): (Option<f64>, Option<f64>, f64) = (None, None, 0.0);
        trace(&mut ctrl, &mut sim, 60.0, |_, s| {
            if (175.0..185.0).contains(&s.x) {
                at_hold = Some(s.y);
            }
            if (300.0..310.0).contains(&s.x) {
                after = Some(s.y);
            }
            over = over.max(s.y - 1.75).max(-1.75 - s.y);
        });
        assert!((at_hold.unwrap() - 1.75).abs() < 0.4, "in the left lane: {:?}", at_hold);
        assert!((after.unwrap() + 1.75).abs() < 0.4, "back right: {:?}", after);
        assert!(over < 0.5, "overshoot {over:.2} m");
    }

    #[test]
    fn stops_where_asked() {
        let mut ctrl = Controller::new(road(400.0), params(), Tuning::default());
        ctrl.set_stop_at(Some(150.0));
        let mut sim = Sim { x: 0.0, y: -1.5, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        trace(&mut ctrl, &mut sim, 60.0, |_, _| {});
        assert!(sim.v.abs() < 0.2, "still moving at {:.2}", sim.v);
        assert!((148.5..=150.5).contains(&sim.x), "stopped at {:.1}", sim.x);
        // Released: it carries on.
        ctrl.set_stop_at(None);
        trace(&mut ctrl, &mut sim, 10.0, |_, _| {});
        assert!(sim.x > 160.0, "did not restart: {:.1}", sim.x);
    }

    #[test]
    fn projects_points_onto_the_route() {
        let mut ctrl = Controller::new(road(400.0), params(), Tuning::default());
        let sim = Sim { x: 50.0, y: -1.5, yaw: 0.0, v: 0.0, steer: 0.0, blocked: false };
        ctrl.step(&sim.obs(), 0.1);
        let o = geo::scale(geo::lonlat_to_unit(0.0, 0.0), R);
        let tan = geo::Tangent::at(o);
        let spot = ctrl.project(sim.obs().pos, tan.unproject(120.0, 2.0), 200.0, 30.0).unwrap();
        assert!((spot.s - 120.0).abs() < 0.5 && (spot.d - 2.0).abs() < 0.2, "{spot:?}");
        assert!((ctrl.s_now() - 50.0).abs() < 0.5);
        let back = ctrl.position_at(120.0, 2.0, R);
        let (bx, by) = tan.project(back);
        assert!((bx - 120.0).abs() < 0.2 && (by - 2.0).abs() < 0.2, "({bx:.2}, {by:.2})");
        assert!(ctrl.project(sim.obs().pos, tan.unproject(120.0, 60.0), 200.0, 30.0).is_none());
    }

    #[test]
    fn steering_left_is_positive() {
        // Target to the north while heading east: turn left.
        let mut ctrl = Controller::new(route(&[(0.0, 0.0), (5.0, 0.0), (5.0, 100.0)], 30.0), params(), Tuning::default());
        let sim = Sim { x: 4.0, y: 0.0, yaw: 0.0, v: 3.0, steer: 0.0, blocked: false };
        match ctrl.step(&sim.obs(), 0.1) {
            Step::Drive(c) => assert!(c.steer > 0.0, "steer {}", c.steer),
            s => panic!("{s:?}"),
        }
    }
}
