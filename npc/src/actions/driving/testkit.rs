//! Test helpers shared by the driving tests: a kinematic bicycle with the
//! game's rate steering, routes in a tangent plane at (0, 0), and a loop
//! driving the bicycle with a controller.
#![cfg(test)]

use super::controller::{Command, Controller, Observation, RoutePt, Step, VehicleParams};
use crate::{horizon::messages::Vec3, world_data::geo};

pub const R: f64 = 6_356_000.0;

pub fn deg(m: f64) -> f64 {
    m / (R * std::f64::consts::PI / 180.0)
}

pub fn params() -> VehicleParams {
    VehicleParams {
        wheelbase_m: 2.0,
        max_steer_rad: 30f64.to_radians(),
        steer_min_ratio: 0.35,
        steer_falloff_kmh: 80.0,
        // The truck's (truck.tscn): slow out, faster back.
        turn_speed_rad_s: 0.2,
        return_speed_rad_s: 1.0,
    }
}

/// Route through lon/lat points (metres from (0,0) east/north), densified
/// every 2 m.
pub fn route(pts: &[(f64, f64)], kmh: f64) -> Vec<(Vec3, f64, f64)> {
    let mut out = Vec::new();
    let mut s = 0.0;
    let mut prev: Option<Vec3> = None;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = (((b.0 - a.0).hypot(b.1 - a.1)) / 2.0).ceil().max(1.0) as usize;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let u = geo::lonlat_to_unit(deg(a.0 + (b.0 - a.0) * t), deg(a.1 + (b.1 - a.1) * t));
            if let Some(p) = prev {
                let d = geo::arc_m(p, u, R);
                if d < 0.01 {
                    continue;
                }
                s += d;
            }
            out.push((u, s, kmh));
            prev = Some(u);
        }
    }
    out
}

/// Kinematic bicycle with the game's rate steering and a crude
/// powertrain, in the tangent plane at (0, 0).
pub struct Sim {
    pub x: f64,
    pub y: f64,
    pub yaw: f64, // CCW from east
    pub v: f64,
    pub steer: f64,
    pub blocked: bool,
}

impl Sim {
    pub fn step(&mut self, c: &Command, p: &VehicleParams, dt: f64) {
        // VehicleSteering.step: out at turn_speed, back at return_speed, and
        // hands-off (dead zone) a drift back to centre growing with speed.
        let lock = p.lock_rad(self.v);
        if c.steer.abs() > 0.01 {
            self.steer += c.steer * p.rate(c.steer, self.steer) * dt;
        } else {
            let drift = 0.3 * (self.v.abs() * 3.6 / 50.0).min(1.0) * dt;
            self.steer -= self.steer.signum() * drift.min(self.steer.abs());
        }
        self.steer = self.steer.clamp(-lock, lock);
        let mut a = if c.throttle >= 0.0 || self.v.abs() < 0.5 {
            3.0 * c.throttle
        } else {
            // service brake
            -6.0 * c.throttle.abs() * self.v.signum()
        };
        if c.brake {
            a -= 7.0 * self.v.signum();
        }
        a -= 0.02 * self.v;
        let nv = self.v + a * dt;
        self.v = if c.brake && nv.signum() != self.v.signum() { 0.0 } else { nv };
        if self.blocked {
            self.v = 0.0;
        }
        self.x += self.v * self.yaw.cos() * dt;
        self.y += self.v * self.yaw.sin() * dt;
        self.yaw += self.v / p.wheelbase_m * self.steer.tan() * dt;
    }

    pub fn obs(&self) -> Observation {
        let o = geo::scale(geo::lonlat_to_unit(0.0, 0.0), R);
        let tan = geo::Tangent::at(o);
        let pos = tan.unproject(self.x, self.y);
        let fwd = geo::add(geo::scale(tan.east, self.yaw.cos()), geo::scale(tan.north, self.yaw.sin()));
        Observation { pos, forward: fwd, speed_mps: self.v, steering_rad: Some(self.steer), age_s: 0.05 }
    }
}

/// A two-lane road (6 m) straight east.
pub fn road(len: f64) -> Vec<RoutePt> {
    route(&[(0.0, 0.0), (len, 0.0)], 40.0)
        .into_iter()
        .map(|(unit, s, kmh)| RoutePt { unit, s, speed_kmh: kmh, half_width_m: 3.0, lanes: 2 })
        .collect()
}

/// Run for `secs` (no early return), recording (x, y) every step.
pub fn trace(ctrl: &mut Controller, sim: &mut Sim, secs: f64, mut each: impl FnMut(&mut Controller, &Sim)) {
    let dt = 0.1;
    let p = params();
    let mut t = 0.0;
    while t < secs {
        each(ctrl, sim);
        if let Step::Drive(c) = ctrl.step(&sim.obs(), dt) {
            for _ in 0..6 {
                sim.step(&c, &p, dt / 6.0);
            }
        }
        t += dt;
    }
}

