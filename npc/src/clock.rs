//! Planetary clock: what hour of the (25 h) planetary day is it, and which day
//! of the rotating shift cycle.
//!
//! Because *1 h real = 1 h game*, wall-clock time **is** planetary time, scaled
//! 1:1; a planetary "day" is just one rotation period. resourcesDynamic derives
//! a planet's rotation phase with **no epoch offset**
//! (`spinAngle = 2π · unix_seconds / (rotation_h · 3600)`), so the hour is a
//! pure function of the Unix clock and matches the engine by construction:
//!
//! ```text
//! planetary_hour = (unix_seconds / 3600) mod rotation_h
//! ```
//!
//! No state is stored — every instance and every restart agrees.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

/// Where we are in the rotating rota: `day` in `0..rota_days`, `hour` in
/// `0..rotation_h`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetTime {
    /// Day index within the rota cycle (`0..rota_days`).
    pub day: u32,
    /// Integer planetary hour (`0..⌈rotation_h⌉`).
    pub hour: u32,
    /// Fractional planetary hour, for sub-hour reasoning / display.
    pub hour_f: f64,
}

/// Process-wide clock parameters, resolved once at startup and shared. The
/// per-planet `rotation_h` lives on [`PlanetClock`], not here.
#[derive(Debug, Clone)]
pub struct ClockParams {
    /// Length of the rota cycle in planetary days (the spreadsheet has 5).
    pub rota_days: u32,
    /// Shifts which rota day "day 0" lands on, so the rota can be aligned.
    pub rota_epoch_offset_days: i64,
    /// Test hook: pin planetary time to this exact Unix second (frozen).
    pub override_unix: Option<f64>,
    /// Test hook: speed up planetary time relative to the process-start anchor.
    /// `1.0` = real time; `3600.0` = one planetary hour per real second.
    pub time_scale: f64,
    /// Real Unix seconds captured at process start (the scaling anchor).
    pub anchor_unix: f64,
}

impl ClockParams {
    /// Capture the process-start anchor. Call once at startup.
    pub fn new(rota_days: u32, rota_epoch_offset_days: i64, override_unix: Option<f64>, time_scale: f64) -> Self {
        Self {
            rota_days: rota_days.max(1),
            rota_epoch_offset_days,
            override_unix,
            time_scale: if time_scale > 0.0 { time_scale } else { 1.0 },
            anchor_unix: unix_now(),
        }
    }

    /// Translate the real Unix clock into the effective planetary Unix seconds,
    /// applying the override / time-scale test hooks.
    fn effective_unix(&self, real_unix: f64) -> f64 {
        match self.override_unix {
            Some(o) => o,
            None => self.anchor_unix + (real_unix - self.anchor_unix) * self.time_scale,
        }
    }
}

/// A clock for one planet: [`ClockParams`] plus that planet's rotation period.
#[derive(Clone)]
pub struct PlanetClock {
    params: Arc<ClockParams>,
    rotation_h: f64,
}

impl PlanetClock {
    pub fn new(params: Arc<ClockParams>, rotation_h: f64) -> Self {
        Self {
            params,
            rotation_h: if rotation_h > 0.0 { rotation_h } else { 25.0 },
        }
    }

    pub fn rotation_h(&self) -> f64 {
        self.rotation_h
    }

    /// Current planetary time from the real wall clock.
    pub fn now(&self) -> PlanetTime {
        self.time_at(unix_now())
    }

    /// Planetary time for a given real Unix second — pure, so it is unit-testable
    /// without touching the system clock.
    pub fn time_at(&self, real_unix: f64) -> PlanetTime {
        let eff = self.params.effective_unix(real_unix);
        let period_s = self.rotation_h * 3600.0;
        let total_days = (eff / period_s).floor();
        let hour_f = (eff - total_days * period_s) / 3600.0;
        let day = ((total_days as i64) + self.params.rota_epoch_offset_days)
            .rem_euclid(self.params.rota_days as i64) as u32;
        PlanetTime {
            day,
            hour: hour_f.floor() as u32,
            hour_f,
        }
    }
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Arc<ClockParams> {
        Arc::new(ClockParams {
            rota_days: 5,
            rota_epoch_offset_days: 0,
            override_unix: None,
            time_scale: 1.0,
            anchor_unix: 0.0,
        })
    }

    #[test]
    fn hour_is_unix_over_3600_mod_rotation() {
        let clk = PlanetClock::new(params(), 25.0);
        // 3 h past a whole number of 25 h days.
        let t = clk.time_at(3.0 * 3600.0);
        assert_eq!(t.hour, 3);
        assert_eq!(t.day, 0);
        // 26 h in -> hour 1 of day 1 (26 mod 25 = 1; 1 whole rotation elapsed).
        let t = clk.time_at(26.0 * 3600.0);
        assert_eq!(t.hour, 1);
        assert_eq!(t.day, 1);
    }

    #[test]
    fn day_wraps_around_the_rota() {
        let clk = PlanetClock::new(params(), 25.0);
        // 5 rotations = one full rota cycle -> back to day 0.
        let t = clk.time_at(5.0 * 25.0 * 3600.0);
        assert_eq!(t.day, 0);
        // 7 rotations -> day 2.
        let t = clk.time_at(7.0 * 25.0 * 3600.0);
        assert_eq!(t.day, 2);
    }

    #[test]
    fn respects_a_different_rotation_period() {
        let clk = PlanetClock::new(params(), 24.0);
        let t = clk.time_at(25.0 * 3600.0);
        assert_eq!(t.hour, 1); // 25 mod 24
        assert_eq!(t.day, 1);
    }

    #[test]
    fn override_freezes_time() {
        let p = Arc::new(ClockParams {
            override_unix: Some(10.0 * 3600.0),
            ..(*params()).clone()
        });
        let clk = PlanetClock::new(p, 25.0);
        // Whatever "real" time we pass, the frozen override wins.
        assert_eq!(clk.time_at(0.0).hour, 10);
        assert_eq!(clk.time_at(999.0 * 3600.0).hour, 10);
    }

    #[test]
    fn time_scale_speeds_up_planetary_time() {
        let p = Arc::new(ClockParams {
            time_scale: 3600.0, // 1 real second = 1 planetary hour
            anchor_unix: 0.0,
            ..(*params()).clone()
        });
        let clk = PlanetClock::new(p, 25.0);
        // 4 real seconds after the anchor -> 4 planetary hours.
        assert_eq!(clk.time_at(4.0).hour, 4);
    }

    #[test]
    fn epoch_offset_shifts_the_rota_day() {
        let p = Arc::new(ClockParams {
            rota_epoch_offset_days: 2,
            ..(*params()).clone()
        });
        let clk = PlanetClock::new(p, 25.0);
        assert_eq!(clk.time_at(0.0).day, 2); // day 0 + offset 2
    }
}
