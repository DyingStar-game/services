//! Game events (component of the column physics): heat and/or dust injected around a point,
//! spread over a duration (each step receives the share of the time window that falls in it,
//! never all at once), journaled in the state so that they survive restarts and the guard's
//! roll-back. The heat goes to the lowest layer (then the convection lifts it), the dust too.

use super::{overlap, ColumnCtx, ColumnProcess};
use crate::pe::{PeModel, PeState};
use crate::vec3::*;
use std::any::Any;

/// A game event: heat and/or dust injected around a point, spread over a duration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub lon_deg: f64,
    pub lat_deg: f64,
    /// Radius of the patch (m); the weight falls from 1 at the centre to 0 at the radius (cos^2).
    pub radius_m: f64,
    /// Start (model time, s) and duration (s) of the injection.
    pub start_s: f64,
    pub duration_s: f64,
    /// Total heating power (W) while the event lasts, given to the lowest layer.
    pub heat_w: f64,
    /// Total dust (kg) over the whole duration, given to the lowest layer.
    pub dust_kg: f64,
    /// Fraction of that dust in the fine class.
    pub fine_frac: f64,
}

impl Event {
    pub const N_FIELDS: usize = 8;
    pub fn to_array(&self) -> [f64; 8] {
        [self.lon_deg, self.lat_deg, self.radius_m, self.start_s, self.duration_s, self.heat_w, self.dust_kg, self.fine_frac]
    }
    pub fn from_array(a: &[f64; 8]) -> Event {
        Event { lon_deg: a[0], lat_deg: a[1], radius_m: a[2], start_s: a[3], duration_s: a[4], heat_w: a[5], dust_kg: a[6], fine_frac: a[7] }
    }
}

/// The events component: spreads the journal over the cells each step, then gives each column
/// its heat and dust.
#[derive(Clone, Debug, Default)]
pub struct Events;

impl ColumnProcess for Events {
    fn name(&self) -> &'static str {
        "events"
    }

    /// Event sources of this step: each event gives the share of its time window that falls in
    /// [t_end - dt, t_end] (sequential, in journal order: deterministic).
    fn prepare(&self, m: &PeModel, st: &PeState, t_end: f64, dt: f64, ev_out: &mut [f64]) {
        for ev in &st.events {
            let ov = overlap(t_end - dt, t_end, ev.start_s, ev.start_s + ev.duration_s);
            if ov <= 0.0 {
                continue;
            }
            for (i, wt) in m.event_weights(ev) {
                ev_out[i * 3] += ev.heat_w * wt * ov;
                ev_out[i * 3 + 1] += ev.dust_kg / ev.duration_s * wt * ov * ev.fine_frac;
                ev_out[i * 3 + 2] += ev.dust_kg / ev.duration_s * wt * ov * (1.0 - ev.fine_frac);
            }
        }
    }

    fn heat(&self, c: &mut ColumnCtx) {
        c.q_low += c.ev[0];
    }

    fn sources(&self, c: &mut ColumnCtx) {
        if c.dust_on {
            let (g, kb, dt) = (c.c.g, c.kb, c.dt);
            c.m[kb] += g * c.ev[1] * dt;
            c.m2[kb] += g * c.ev[2] * dt;
            c.bud[3] = (c.ev[1] + c.ev[2]) * dt;
        } else {
            c.bud[3] = 0.0;
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl PeModel {
    /// Adds a game event to the journal (it acts from its start time on, spread over its duration).
    pub fn inject_event(&self, st: &mut PeState, ev: Event) -> Result<(), String> {
        if !(ev.duration_s > 0.0 && ev.radius_m > 0.0 && ev.heat_w >= 0.0 && ev.dust_kg >= 0.0 && (0.0..=1.0).contains(&ev.fine_frac)) {
            return Err(format!("invalid event {ev:?}"));
        }
        st.events.push(ev);
        Ok(())
    }

    /// Per-cell weights (sum of area x weight = 1) of an event patch, in cell order.
    pub fn event_weights(&self, ev: &Event) -> Vec<(usize, f64)> {
        let gr = &self.grid;
        let c = from_lonlat(ev.lon_deg.to_radians(), ev.lat_deg.to_radians());
        let mut w: Vec<(usize, f64)> = Vec::new();
        for i in 0..gr.n_cells() {
            let d = angle(gr.x[i], c) * gr.radius;
            if d < ev.radius_m {
                let s = (std::f64::consts::FRAC_PI_2 * d / ev.radius_m).cos();
                w.push((i, s * s));
            }
        }
        if w.is_empty() {
            w.push((gr.nearest(c, 0), 1.0));
        }
        let norm: f64 = w.iter().map(|&(i, s)| gr.area[i] * s).sum();
        w.iter().map(|&(i, s)| (i, s / norm)).collect()
    }
}
