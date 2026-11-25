//! Rotating shallow-water equations on the hexagonal grid, finite volumes,
//! collocated ("A-grid") unknowns, velocity stored as a 3-D Cartesian vector
//! tangent to the sphere.
//!
//! Equations (vector-invariant momentum, flux-form mass and tracer):
//!   dh/dt  = -div(h v)                                  + hyperdiffusion of (h + b)
//!   dv/dt  = -(zeta + f) r x v - grad(g (h + b) + |v|^2/2) + hyperdiffusion of v
//!   dm/dt  = -div(h v c),  m = h c  (passive tracer, column amount)
//!
//! Discrete operators (Voronoi cells, faces between neighbours):
//!   div  : Gauss theorem, face flux = mean of the two cells (antisymmetric => exact mass conservation)
//!   curl : Stokes theorem around the cell, face velocity = mean of the two cells
//!   grad : Gauss theorem on (phi_face - phi_cell), projected on the tangent plane
//!   lap  : standard Voronoi Laplacian (phi_j - phi_i) l / d, used twice for del^4
//!   tracer face value: upwind-biased "kappa = 1/3" reconstruction clipped to the
//!   upwind neighbourhood range, then an outflow limiter that guarantees m >= 0 for every forward-Euler stage.
//! Time stepping: SSP-RK3 (Shu-Osher), a convex combination of forward-Euler stages,
//! so the positivity of the tracer carries over to the full step.

use crate::grid::Grid;
use crate::vec3::*;
use rayon::prelude::*;

#[derive(Clone)]
pub struct State {
    /// Layer depth (m).
    pub h: Vec<f64>,
    /// Velocity (m/s), Cartesian, tangent to the sphere.
    pub v: Vec<V3>,
    /// Tracer column amount m = h * c (c = mixing ratio).
    pub m: Vec<f64>,
    /// Model time (s).
    pub time: f64,
}

impl State {
    pub fn zeros(n: usize) -> State {
        State { h: vec![0.0; n], v: vec![[0.0; 3]; n], m: vec![0.0; n], time: 0.0 }
    }
}

pub struct Model {
    pub grid: Grid,
    pub g: f64,
    /// Coriolis parameter per cell (1/s).
    pub f: Vec<f64>,
    /// Bottom topography (m).
    pub b: Vec<f64>,
    /// Hyperdiffusion coefficient (m^4/s).
    pub nu4: f64,
    /// When false, h and v are frozen (prescribed wind, tracer only).
    pub dynamics: bool,
    /// Optional prescribed face volume fluxes (v.n * l, m^2/s), e.g. from a stream function.
    pub prescribed: Option<Vec<f64>>,
    /// Planet rotation rate, only used for the angular-momentum diagnostic.
    pub omega_for_diag: f64,
}

#[derive(Clone, Copy, Default)]
struct CellC {
    dh0: f64,
    dv0: V3,
    lap1: [f64; 4],
    gc: V3,
    cmin: f64,
    cmax: f64,
}

pub struct Tend {
    dh: Vec<f64>,
    dv: Vec<V3>,
    dm: Vec<f64>,
}

/// Scratch buffers, allocated once.
pub struct Work {
    bern: Vec<f64>,
    eta: Vec<f64>,
    c: Vec<f64>,
    ea: Vec<[f64; 8]>,
    cc: Vec<CellC>,
    ed: Vec<[f64; 5]>,
    ratio: Vec<f64>,
    tlim: Vec<f64>,
    k: Tend,
    s1: State,
    s2: State,
}

impl Work {
    pub fn new(n: usize, ne: usize) -> Work {
        Work {
            bern: vec![0.0; n],
            eta: vec![0.0; n],
            c: vec![0.0; n],
            ea: vec![[0.0; 8]; ne],
            cc: vec![CellC::default(); n],
            ed: vec![[0.0; 5]; ne],
            ratio: vec![1.0; n],
            tlim: vec![0.0; ne],
            k: Tend { dh: vec![0.0; n], dv: vec![[0.0; 3]; n], dm: vec![0.0; n] },
            s1: State::zeros(n),
            s2: State::zeros(n),
        }
    }
}

/// Damping e-folding time (s) of the shortest grid mode -> del^4 coefficient.
/// For a hexagonal Voronoi Laplacian the largest eigenvalue is about 6 / d^2.
pub fn nu4_from_tau(grid: &Grid, tau: f64) -> f64 {
    let d = grid.mean_spacing();
    d.powi(4) / (36.0 * tau)
}

#[derive(Clone, Copy, Debug)]
pub struct Diag {
    pub mass: f64,
    pub tracer: f64,
    pub energy: f64,
    pub min_h: f64,
    pub min_m: f64,
    pub max_speed: f64,
    /// Axial absolute angular momentum (kg-free: m^5/s, i.e. per unit density).
    pub ang_mom: f64,
    pub finite: bool,
}

impl Model {
    pub fn new_work(&self) -> Work {
        Work::new(self.grid.n_cells(), self.grid.n_edges())
    }

    fn rhs(&self, st: &State, dt: f64, w: &mut Work) {
        let gr = &self.grid;
        let (h, v, m) = (&st.h, &st.v, &st.m);
        let g = self.g;
        let b = &self.b;

        // Pass A (cells): Bernoulli function, free surface, mixing ratio.
        w.bern
            .par_iter_mut()
            .zip(w.eta.par_iter_mut())
            .zip(w.c.par_iter_mut())
            .enumerate()
            .for_each(|(i, ((bn, et), c))| {
                *et = h[i] + b[i];
                *bn = g * *et + 0.5 * dot(v[i], v[i]);
                *c = if h[i] > 0.0 { m[i] / h[i] } else { 0.0 };
            });

        // Pass B (edges): mass flux, circulation, Bernoulli and Laplacian face terms.
        let (bern, eta, c) = (&w.bern, &w.eta, &w.c);
        let pres = self.prescribed.as_deref();
        w.ea.par_iter_mut().enumerate().for_each(|(e, o)| {
            let (i, j) = (gr.e_i[e], gr.e_j[e]);
            let l = gr.e_l[e];
            let ld = l / gr.e_d[e];
            let he = 0.5 * (h[i] + h[j]);
            let ve = scale(add(v[i], v[j]), 0.5);
            let unl = match pres {
                Some(p) => p[e],
                None => dot(ve, gr.e_n[e]) * l,
            };
            o[0] = he * unl;
            o[1] = dot(ve, gr.e_t[e]) * l;
            o[2] = 0.5 * (bern[j] - bern[i]) * l;
            o[3] = (eta[j] - eta[i]) * ld;
            o[4] = (v[j][0] - v[i][0]) * ld;
            o[5] = (v[j][1] - v[i][1]) * ld;
            o[6] = (v[j][2] - v[i][2]) * ld;
            o[7] = 0.5 * (c[j] - c[i]) * l;
        });

        // Pass C (cells): divergence, vorticity, gradients, first Laplacian, limiter.
        let ea = &w.ea;
        let f = &self.f;
        w.cc.par_iter_mut().enumerate().for_each(|(i, o)| {
            let inv = 1.0 / gr.area[i];
            let mut div = 0.0;
            let mut circ = 0.0;
            let mut gb = [0.0; 3];
            let mut gc = [0.0; 3];
            let mut lap = [0.0; 4];
            let (mut cmin, mut cmax) = (c[i], c[i]);
            let (s0, s1) = (gr.c_start[i], gr.c_start[i + 1]);
            for s in s0..s1 {
                let e = gr.c_edge[s];
                let sg = gr.c_sign[s];
                let a = &ea[e];
                let n = gr.e_n[e];
                div += sg * a[0];
                circ += sg * a[1];
                gb = add(gb, scale(n, a[2]));
                gc = add(gc, scale(n, a[7]));
                for k in 0..4 {
                    lap[k] += sg * a[3 + k];
                }
                let cn = c[gr.c_nbr[s]];
                cmin = cmin.min(cn);
                cmax = cmax.max(cn);
            }
            let x = gr.x[i];
            let zeta = circ * inv;
            let gb = tangent(scale(gb, inv), x);
            let gc = tangent(scale(gc, inv), x);
            let rxv = cross(x, v[i]);
            o.dh0 = -div * inv;
            o.dv0 = sub(scale(rxv, -(zeta + f[i])), gb);
            for k in 0..4 {
                o.lap1[k] = lap[k] * inv;
            }
            o.gc = gc;
            o.cmin = cmin;
            o.cmax = cmax;
        });

        // Pass D (edges): second Laplacian fluxes, upwind-biased tracer flux.
        let cc = &w.cc;
        w.ed.par_iter_mut().enumerate().for_each(|(e, o)| {
            let (i, j) = (gr.e_i[e], gr.e_j[e]);
            let ld = gr.e_l[e] / gr.e_d[e];
            for k in 0..4 {
                o[k] = (cc[j].lap1[k] - cc[i].lap1[k]) * ld;
            }
            let fl = ea[e][0];
            let (u, dn, slot) = if fl >= 0.0 { (i, j, gr.e_slot_i[e]) } else { (j, i, gr.e_slot_j[e]) };
            // "kappa = 1/3" face value (third-order upwind-biased in 1-D), written with
            // the upwind cell gradient and the downwind neighbour, then clipped to the
            // range of the upwind cell's neighbourhood (no new extrema, no undershoot).
            let raw = c[u] + (2.0 / 3.0) * dot(cc[u].gc, gr.c_r[slot]) + (c[dn] - c[u]) / 6.0;
            let cf = raw.clamp(cc[u].cmin, cc[u].cmax);
            o[4] = fl * cf.max(0.0);
        });

        // Pass E (cells): del^4, final h / v tendencies, tracer outflow limiter.
        let ed = &w.ed;
        let nu = self.nu4;
        let dynamics = self.dynamics;
        w.k.dh
            .par_iter_mut()
            .zip(w.k.dv.par_iter_mut())
            .zip(w.ratio.par_iter_mut())
            .enumerate()
            .for_each(|(i, ((dh, dv), ratio))| {
                let inv = 1.0 / gr.area[i];
                let mut d4 = [0.0; 4];
                let mut out = 0.0;
                for s in gr.c_start[i]..gr.c_start[i + 1] {
                    let e = gr.c_edge[s];
                    let sg = gr.c_sign[s];
                    for k in 0..4 {
                        d4[k] += sg * ed[e][k];
                    }
                    out += (sg * ed[e][4]).max(0.0);
                }
                if dynamics {
                    *dh = cc[i].dh0 - nu * d4[0] * inv;
                    let dvv = sub(cc[i].dv0, scale([d4[1], d4[2], d4[3]], nu * inv));
                    *dv = tangent(dvv, gr.x[i]);
                } else {
                    *dh = 0.0;
                    *dv = [0.0; 3];
                }
                // A cell may not export more than it holds during this stage; the
                // small safety margin keeps round-off from producing -1e-60 values.
                let loss = out * dt * inv;
                let avail = m[i].max(0.0) * (1.0 - 1e-10);
                *ratio = if loss > avail { avail / loss } else { 1.0 };
            });

        // Pass F (edges): scale outgoing tracer fluxes by the upwind cell's ratio.
        let ratio = &w.ratio;
        w.tlim.par_iter_mut().enumerate().for_each(|(e, t)| {
            let tr = ed[e][4];
            let u = if tr >= 0.0 { gr.e_i[e] } else { gr.e_j[e] };
            *t = tr * ratio[u];
        });

        // Pass G (cells): tracer tendency.
        let tlim = &w.tlim;
        w.k.dm.par_iter_mut().enumerate().for_each(|(i, dm)| {
            let mut s_ = 0.0;
            for s in gr.c_start[i]..gr.c_start[i + 1] {
                s_ += gr.c_sign[s] * tlim[gr.c_edge[s]];
            }
            *dm = -s_ / gr.area[i];
        });
    }

    /// out = a * x + c * (y + dt * k)
    fn combine(out: &mut State, a: f64, x: &State, c: f64, y: &State, dt: f64, k: &Tend) {
        out.h.par_iter_mut().enumerate().for_each(|(i, o)| {
            *o = a * x.h[i] + c * (y.h[i] + dt * k.dh[i]);
        });
        out.m.par_iter_mut().enumerate().for_each(|(i, o)| {
            *o = a * x.m[i] + c * (y.m[i] + dt * k.dm[i]);
        });
        out.v.par_iter_mut().enumerate().for_each(|(i, o)| {
            *o = add(scale(x.v[i], a), scale(add(y.v[i], scale(k.dv[i], dt)), c));
        });
    }

    /// One SSP-RK3 step of length dt.
    pub fn step(&self, st: &mut State, dt: f64, w: &mut Work) {
        self.rhs(st, dt, w);
        let mut s1 = std::mem::replace(&mut w.s1, State::zeros(0));
        let mut s2 = std::mem::replace(&mut w.s2, State::zeros(0));
        Self::combine(&mut s1, 0.0, st, 1.0, st, dt, &w.k);
        self.rhs(&s1, dt, w);
        Self::combine(&mut s2, 0.75, st, 0.25, &s1, dt, &w.k);
        self.rhs(&s2, dt, w);
        // In place: st = 1/3 st + 2/3 (s2 + dt k)
        let k = &w.k;
        st.h.par_iter_mut().enumerate().for_each(|(i, o)| {
            *o = *o / 3.0 + 2.0 / 3.0 * (s2.h[i] + dt * k.dh[i]);
        });
        st.m.par_iter_mut().enumerate().for_each(|(i, o)| {
            *o = *o / 3.0 + 2.0 / 3.0 * (s2.m[i] + dt * k.dm[i]);
        });
        st.v.par_iter_mut().enumerate().for_each(|(i, o)| {
            *o = add(scale(*o, 1.0 / 3.0), scale(add(s2.v[i], scale(k.dv[i], dt)), 2.0 / 3.0));
        });
        st.time += dt;
        w.s1 = s1;
        w.s2 = s2;
    }

    /// CFL time step: courant * centre distance / (gravity-wave speed + wind speed).
    pub fn cfl_dt(&self, st: &State, courant: f64) -> f64 {
        let gr = &self.grid;
        let g = self.g;
        let dynamics = self.dynamics;
        let speed = |i: usize| {
            let s = dot(st.v[i], st.v[i]).sqrt();
            if dynamics {
                s + (g * st.h[i].max(0.0)).sqrt()
            } else {
                s
            }
        };
        let r = (0..gr.n_edges())
            .into_par_iter()
            .map(|e| {
                let sp = speed(gr.e_i[e]).max(speed(gr.e_j[e])).max(1e-12);
                gr.e_d[e] / sp
            })
            .reduce(|| f64::MAX, f64::min);
        courant * r
    }

    pub fn diag(&self, st: &State) -> Diag {
        let gr = &self.grid;
        let g = self.g;
        let mut d = Diag {
            mass: 0.0,
            tracer: 0.0,
            energy: 0.0,
            min_h: f64::MAX,
            min_m: f64::MAX,
            max_speed: 0.0,
            ang_mom: 0.0,
            finite: true,
        };
        let (r, om) = (gr.radius, self.omega_for_diag);
        for i in 0..gr.n_cells() {
            let a = gr.area[i];
            let h = st.h[i];
            let k = 0.5 * dot(st.v[i], st.v[i]);
            d.mass += a * h;
            d.tracer += a * st.m[i];
            d.energy += a * (h * k + g * h * (0.5 * h + self.b[i]));
            d.min_h = d.min_h.min(h);
            d.min_m = d.min_m.min(st.m[i]);
            d.max_speed = d.max_speed.max((2.0 * k).sqrt());
            let x = gr.x[i];
            let rel = cross(x, st.v[i])[2];
            d.ang_mom += a * h * r * (rel + om * r * (x[0] * x[0] + x[1] * x[1]));
            if !(h.is_finite() && k.is_finite() && st.m[i].is_finite()) {
                d.finite = false;
            }
        }
        d
    }

    /// Runs until `t_end` with a CFL-limited step, landing exactly on `t_end`.
    /// Returns the number of steps, or an error message on blow-up.
    pub fn run_until(&self, st: &mut State, t_end: f64, courant: f64, w: &mut Work) -> Result<usize, String> {
        let mut n = 0;
        while st.time < t_end - 1e-9 {
            let mut dt = self.cfl_dt(st, courant);
            let left = t_end - st.time;
            if dt >= left {
                dt = left;
            } else if dt > 0.5 * left {
                dt = 0.5 * left; // avoid a tiny final step
            }
            self.step(st, dt, w);
            n += 1;
            if n % 200 == 0 {
                let ok = st.h.iter().all(|x| x.is_finite()) && (!self.dynamics || st.h.iter().all(|&x| x > 0.0));
                if !ok {
                    return Err(format!("blow-up or dry cell at t = {:.0} s", st.time));
                }
            }
        }
        let ok = st.h.iter().all(|x| x.is_finite()) && st.v.iter().all(|x| x[0].is_finite());
        if !ok {
            return Err(format!("non-finite values at t = {:.0} s", st.time));
        }
        Ok(n)
    }
}
