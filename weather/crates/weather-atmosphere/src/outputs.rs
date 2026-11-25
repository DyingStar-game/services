//! Outputs of the primitive-equation runs: the 16-bit PNG tables of each instant (with the
//! published "ingredients"), the vertical cross-sections, the hourly and planet-wide records of
//! the column physics, the compact climate file and its header. Nothing here changes the model
//! state; everything is in model days of the body.

use crate::io::{self, Sampler, TableMeta};
use crate::pe::*;
use crate::phys::dust::DustLifting;
use crate::phys::sea::DustSea;
use crate::setup::Scenario;
use crate::vec3::*;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

// ------------------------------------------------------------------ diagnostics on a lat-lon grid

/// Sea-level pressure (Pa): the temperature of the lowest layer is extrapolated to the ground
/// and then to the reference sphere with the reference profile's lapse rate (the usual reduction).
pub fn sea_level_pressure(model: &PeModel, st: &PeState, relief: &[f64]) -> Vec<f64> {
    let c = &model.c;
    let l = model.nl;
    let gam = model.tref.lapse.max(1e-4);
    let ex = c.rd * gam / c.g;
    (0..model.grid.n_cells())
        .map(|i| {
            let tl = model.temperature(st, i, l - 1);
            let ts = tl * (model.p_full(l - 1, st.ps[i]) / st.ps[i]).powf(-ex);
            let tmsl = ts + gam * relief[i];
            st.ps[i] * (tmsl / ts).powf(c.g / (c.rd * gam))
        })
        .collect()
}

pub struct Fields {
    pub t: Vec<Vec<f64>>,
    pub u: Vec<Vec<V3>>,
}

pub fn layer_fields(model: &PeModel, st: &PeState) -> Fields {
    let l = model.nl;
    let n = model.grid.n_cells();
    let t = (0..l).map(|k| (0..n).map(|i| model.temperature(st, i, k)).collect()).collect();
    let u = (0..l).map(|k| (0..n).map(|i| st.v[i * l + k]).collect()).collect();
    Fields { t, u }
}

/// Column dust relative to a full column released at the mean surface pressure: sum_k q_k dp_k / p0.
pub fn dust_column(model: &PeModel, st: &PeState) -> Vec<f64> {
    let l = model.nl;
    (0..model.grid.n_cells()).map(|i| (0..l).map(|k| st.m[i * l + k]).sum::<f64>() / model.c.p0).collect()
}

/// Mid layer used by the outputs: the one closest to sigma = 0.5.
pub fn mid_layer(model: &PeModel) -> usize {
    (0..model.nl)
        .min_by(|&a, &b| (model.sig_full[a] - 0.5).abs().partial_cmp(&(model.sig_full[b] - 0.5).abs()).unwrap())
        .unwrap()
}

/// One output table on the image grid of a sampler, before encoding. The names are the ones of
/// the prototype's files (French: `pmer`, `u_bas`, `poussiere`...), kept so that the R&D tools and
/// the published notes still match.
pub struct TableField {
    pub name: &'static str,
    pub units: &'static str,
    pub description: &'static str,
    pub values: Vec<f32>,
}

/// Every table of an instant: pressures, winds and temperatures of the lowest and mid layers, the
/// dust, and with the column physics the published "ingredients" (needs the work buffers of the
/// last step, where the physics left its diagnostics).
pub fn table_fields(model: &PeModel, st: &PeState, relief: &[f64], smp: &Sampler, work: Option<&PeWork>) -> Vec<TableField> {
    let l = model.nl;
    let fl = layer_fields(model, st);
    let km = mid_layer(model);
    let kb = l - 1;
    let mut out = Vec::with_capacity(21);
    let mut push = |name: &'static str, units: &'static str, description: &'static str, values: Vec<f32>| out.push(TableField { name, units, description, values });
    let slp: Vec<f64> = sea_level_pressure(model, st, relief).iter().map(|p| p / 100.0).collect();
    push("pmer", "hPa", "pressure reduced to the reference sphere (lowest-layer temperature extrapolated with the reference lapse rate)", smp.scalar(&slp));
    let psh: Vec<f64> = st.ps.iter().map(|p| p / 100.0).collect();
    push("ps", "hPa", "surface pressure", smp.scalar(&psh));
    let (ub, vb) = smp.east_north(&fl.u[kb]);
    let (um, vm) = smp.east_north(&fl.u[km]);
    push("u_bas", "m/s", "eastward wind, lowest layer", ub);
    push("v_bas", "m/s", "northward wind, lowest layer", vb);
    push("t_bas", "K", "temperature, lowest layer", smp.scalar(&fl.t[kb]));
    push("u_mil", "m/s", "eastward wind, mid layer (sigma ~ 0.5)", um);
    push("v_mil", "m/s", "northward wind, mid layer (sigma ~ 0.5)", vm);
    push("t_mil", "K", "temperature, mid layer (sigma ~ 0.5)", smp.scalar(&fl.t[km]));
    if model.phys.is_some() && work.is_none() {
        // Column physics without its diagnostics (no step since the start): no ingredients.
    } else if let (Some(_), Some(wk)) = (&model.phys, work) {
        use crate::phys::di;
        let d = |q: usize| wk.ph.get(q);
        push("poussiere", "1", "dust optical depth of the column (visible, both classes)", smp.scalar(&d(di::TAU_DUST)));
        let tab: [(&'static str, &'static str, &'static str, Vec<f64>); 12] = [
            ("flux_chaleur", "W/m2", "ingredient: sensible heat flux from the ground to the air (positive upward)", d(di::H)),
            ("ecart_sol_air", "K", "ingredient: ground skin temperature minus surface-air temperature", d(di::DT_GA)),
            ("couche_limite", "m", "ingredient: boundary-layer depth above the ground (parcel method by day, Zilitinkevich depth at night)", d(di::ZI)),
            (
                "refroid_nuit",
                "K/h",
                "ingredient: night-time cooling rate of the ground (0 where the sun is up)",
                d(di::DTG).iter().zip(d(di::COSZ)).map(|(r, cz)| if cz <= 0.0 { (-r).max(0.0) } else { 0.0 }).collect(),
            ),
            ("tension_vent", "Pa", "ingredient: surface wind stress rho u*^2 (gusts included)", d(di::TAU)),
            ("soulevement", "mg/m2/s", "ingredient: dust lifting rate by the wind", d(di::LIFT).iter().map(|x| x * 1e6).collect()),
            ("couvercle", "K", "ingredient: lid strength, potential temperature 500 m above minus 500 m below the lid bottom (0 where the ground is too high)", d(di::LID)),
            ("u_etoile", "m/s", "friction velocity u*", d(di::USTAR)),
            ("w_etoile", "m/s", "convective velocity w* (0 when the ground is not heating the air)", d(di::WSTAR)),
            ("t_sol", "K", "ground skin temperature", d(di::TG)),
            ("poussiere_fine", "g/m2", "fine dust column (1.5 micrometres)", d(di::COL_FINE).iter().map(|x| x * 1e3).collect()),
            ("poussiere_grosse", "g/m2", "coarse dust column (10 micrometres)", d(di::COL_COARSE).iter().map(|x| x * 1e3).collect()),
        ];
        for (name, units, desc, vals) in tab {
            push(name, units, desc, smp.scalar(&vals));
        }
    } else {
        push("poussiere", "1", "dust column relative to a full column released at the mean surface pressure (sum q dp / p0)", smp.scalar(&dust_column(model, st)));
    }
    out
}

/// The metadata of a table of this instant (JSON sidecar).
fn table_meta<'a>(model: &PeModel, st: &PeState, day_s: f64, f: &'a TableField) -> TableMeta<'a> {
    TableMeta {
        field: f.name,
        units: f.units,
        description: f.description,
        time_s: st.time,
        time_hours: st.time / 3600.0,
        planet_day_s: day_s,
        grid_level: model.grid.level,
        grid_cells: model.grid.n_cells(),
        radius_m: model.grid.radius,
        fixed_range: None,
    }
}

/// Every table of an instant, encoded in memory (16-bit PNG + JSON sidecar), by name.
pub fn render_tables(model: &PeModel, st: &PeState, relief: &[f64], smp: &Sampler, work: Option<&PeWork>, day_s: f64) -> std::io::Result<Vec<(&'static str, io::EncodedTable)>> {
    table_fields(model, st, relief, smp, work)
        .iter()
        .map(|f| io::encode_table(smp.w, smp.h, &f.values, &table_meta(model, st, day_s, f)).map(|t| (f.name, t)))
        .collect()
}

/// Writes every table of an instant to `dir` (`t<hours>h_<name>.png` + `.json`), and with the
/// column physics the vertical cross-sections.
pub fn write_tables(model: &PeModel, st: &PeState, relief: &[f64], smp: &Sampler, dir: &Path, work: Option<&PeWork>, sc: &Scenario) -> std::io::Result<()> {
    let stem = format!("t{:06}h", (st.time / 3600.0).round() as i64);
    for f in table_fields(model, st, relief, smp, work) {
        io::write_table(&dir.join(format!("{stem}_{}.png", f.name)), smp.w, smp.h, &f.values, &table_meta(model, st, sc.body.day_s, &f))?;
    }
    if let (Some(_), Some(_)) = (&model.phys, work) {
        write_sections(model, st, dir, &stem, &sc.s.outputs.section_lats)?;
    }
    Ok(())
}

/// Vertical cross-sections along the given latitudes (nearest cell every degree of longitude):
/// ground altitude, then per layer (top to ground) altitude of the full level, fine and coarse
/// dust (mg/kg), potential temperature (K) and wind speed (m/s). File tags: "20N", "35S", "eq".
fn write_sections(model: &PeModel, st: &PeState, dir: &Path, stem: &str, lats: &[f64]) -> std::io::Result<()> {
    let sdir = dir.join("coupes");
    std::fs::create_dir_all(&sdir)?;
    let l = model.nl;
    for &lat in lats {
        let tag = if lat == 0.0 { "eq".to_string() } else { format!("{:.0}{}", lat.abs(), if lat > 0.0 { "N" } else { "S" }) };
        let mut s = String::from("lon,z_sol");
        for k in 0..l {
            let _ = write!(s, ",z{k},qf{k},qc{k},th{k},v{k}");
        }
        s.push('\n');
        let mut start = 0;
        for c in 0..360 {
            let lon = -179.5 + c as f64;
            let i = model.grid.nearest(from_lonlat(lon.to_radians(), lat.to_radians()), start);
            start = i;
            let (zh, zf) = model.column_heights(st, i);
            let _ = write!(s, "{lon:.1},{:.1}", zh[l]);
            for k in 0..l {
                let dp = model.dp(k, st.ps[i]);
                let th = st.th[i * l + k] / dp;
                let _ = write!(
                    s,
                    ",{:.1},{:.5},{:.5},{:.2},{:.2}",
                    zf[k],
                    st.m[i * l + k] / dp * 1e6,
                    st.m2[i * l + k] / dp * 1e6,
                    th,
                    norm(st.v[i * l + k])
                );
            }
            s.push('\n');
        }
        std::fs::write(sdir.join(format!("{stem}_coupe_{tag}.csv")), s)?;
    }
    Ok(())
}

/// Recording of the column-physics diagnostics during the recorded period.
pub struct PhysRecorder {
    day_s: f64,
    points: Vec<(String, usize)>,
    pts_csv: std::fs::File,
    dust_csv: std::fs::File,
    prof_csv: Option<std::fs::File>,
    /// Lifting threshold (Pa; infinite without lifting), overhead sunlight (W/m^2).
    tau_t: f64,
    s0: f64,
    /// Dust sea (if any): target, base and top, top of the band above; profile bins.
    sea: Option<(f64, f64, f64, f64)>,
    prof: (f64, f64, usize),
    /// Altitude classes of the summary (m).
    classes: [f64; 3],
    // Per-cell annual accumulators (hourly samples).
    hours: u64,
    n_exceed: Vec<f64>,
    sum_tau: Vec<f64>,
    sum_ta: Vec<f64>,
    sum_h: Vec<f64>,
    /// Planet-wide sums (area-weighted, hourly) of the sunlight at the top, absorbed by the
    /// ground and absorbed by the dust: the planetary albedo.
    sw_sums: [f64; 3],
    sum_tg_day: Vec<f64>,
    n_day: Vec<f64>,
    sum_tg_night: Vec<f64>,
    n_night: Vec<f64>,
    day_max_tg: Vec<f64>,
    day_min_tg: Vec<f64>,
    day_max_ta: Vec<f64>,
    day_min_ta: Vec<f64>,
    day_max_zi: Vec<f64>,
    sum_range_tg: Vec<f64>,
    sum_range_ta: Vec<f64>,
    sum_zi_max: Vec<f64>,
    days: f64,
    // Planet-wide time series for the summary.
    band_sea: Vec<f64>,
    band_above: Vec<f64>,
    basin: Vec<f64>,
    sea_base: Vec<f64>,
    sea_top: Vec<f64>,
    col_base: Vec<f64>,
    col_touch: Vec<f64>,
    col_top: Vec<f64>,
    tau_mean: Vec<f64>,
    lift_frac: Vec<f64>,
}

impl PhysRecorder {
    pub fn new(model: &PeModel, relief: &[f64], dir: &Path, append: bool, sc: &Scenario) -> std::io::Result<PhysRecorder> {
        let gr = &model.grid;
        let n = gr.n_cells();
        let ph = model.phys.as_ref().expect("physics recorder without physics");
        let tropics = |i: usize| gr.lat[i].to_degrees().abs() < 30.0;
        let low = (0..n).filter(|&i| tropics(i)).min_by(|&a, &b| relief[a].partial_cmp(&relief[b]).unwrap()).unwrap();
        let high = (0..n).filter(|&i| tropics(i)).max_by(|&a, &b| relief[a].partial_cmp(&relief[b]).unwrap()).unwrap();
        let near = |lo: f64, la: f64| gr.nearest(from_lonlat(lo.to_radians(), la.to_radians()), 0);
        let mut points = vec![("fond_bassin".to_string(), low), ("haut_plateau".to_string(), high)];
        for p in &sc.s.outputs.tracked_points {
            points.push((p.name.clone(), near(p.lon, p.lat)));
        }
        let open = |name: &str, head: &str| -> std::io::Result<std::fs::File> {
            let p = dir.join(name);
            let fresh = !append || !p.exists();
            let mut f = std::fs::OpenOptions::new().create(true).append(!fresh).write(true).truncate(fresh).open(p)?;
            if fresh {
                f.write_all(head.as_bytes())?;
            }
            Ok(f)
        };
        let mut pts_head = String::from("# points: ");
        for (nm, i) in &points {
            let _ = write!(pts_head, "{nm} = cell {i} (lon {:.1}, lat {:.1}, relief {:.0} m); ", gr.lon[*i].to_degrees(), gr.lat[*i].to_degrees(), relief[*i]);
        }
        pts_head.push_str("\nday,point,relief_m,t_sol_K,t_air_K,flux_chaleur_Wm2,couche_limite_m,tension_Pa,soulevement_mgm2s,tau_poussiere,cos_zenith,sw_sol_Wm2,vent_bas_ms,couvercle_K\n");
        let pts_csv = open("points_horaires.csv", &pts_head)?;
        let ss = &sc.s.sea;
        let sea = ph.find::<DustSea>().map(|d| (d.q_target, d.z_base, d.z_top, ss.above_band_top_m));
        let (zb, zt, za) = sea.map(|x| (x.1, x.2, x.3)).unwrap_or((f64::NAN, f64::NAN, f64::NAN));
        let dust_csv = open(
            "poussiere.csv",
            &format!(
                "day,tau_moyen,tau_mediane,tau_p90,tau_max,part_surface_soulevement,soulevement_total_kg_s,mer_base_m,mer_sommet_m,vent_mer_{zb:.0}_{zt:.0}_ms,vent_dessus_{zt:.0}_{za:.0}_ms,vent_bas_bassins_ms,couvercle_moyen_K,mer_base_colonnes_m,part_colonnes_poussiere_au_sol,mer_sommet_colonnes_m,masse_fine_kg,masse_grosse_kg,cumul_souleve_kg,cumul_depose_kg,cumul_mer_kg,cumul_evenements_kg\n"
            ),
        )?;
        let prof = (ss.profile_bottom_m, ss.profile_step_m, ss.profile_bins);
        let prof_csv = if sea.is_some() {
            let mut h = String::from("day");
            for b in 0..prof.2 {
                let _ = write!(h, ",z{:.0}", prof.0 + (b as f64 + 0.5) * prof.1);
            }
            h.push('\n');
            Some(open("mer_profils.csv", &h)?)
        } else {
            None
        };
        let z = || vec![0.0; n];
        Ok(PhysRecorder {
            day_s: sc.body.day_s,
            points,
            pts_csv,
            dust_csv,
            prof_csv,
            tau_t: ph.find::<DustLifting>().map(|d| d.tau_t).unwrap_or(f64::INFINITY),
            s0: ph.sun.s0,
            sea,
            prof,
            classes: sc.s.outputs.altitude_classes_m,
            hours: 0,
            n_exceed: z(),
            sum_tau: z(),
            sum_ta: z(),
            sum_h: z(),
            sw_sums: [0.0; 3],
            sum_tg_day: z(),
            n_day: z(),
            sum_tg_night: z(),
            n_night: z(),
            day_max_tg: vec![f64::MIN; n],
            day_min_tg: vec![f64::MAX; n],
            day_max_ta: vec![f64::MIN; n],
            day_min_ta: vec![f64::MAX; n],
            day_max_zi: z(),
            sum_range_tg: z(),
            sum_range_ta: z(),
            sum_zi_max: z(),
            days: 0.0,
            band_sea: Vec::new(),
            band_above: Vec::new(),
            basin: Vec::new(),
            sea_base: Vec::new(),
            sea_top: Vec::new(),
            col_base: Vec::new(),
            col_touch: Vec::new(),
            col_top: Vec::new(),
            tau_mean: Vec::new(),
            lift_frac: Vec::new(),
        })
    }

    /// Hourly sample (end of every chunk of the recorded period).
    pub fn hourly(&mut self, model: &PeModel, st: &PeState, w: &PeWork, relief: &[f64]) -> std::io::Result<()> {
        use crate::phys::di;
        let l = model.nl;
        let dg = &w.ph.diag;
        let (tau_t, s0) = (self.tau_t, self.s0);
        let mut s = String::new();
        for (nm, i) in &self.points {
            let r = &dg[i * di::N..(i + 1) * di::N];
            let _ = writeln!(
                s,
                "{:.4},{nm},{:.0},{:.2},{:.2},{:.1},{:.0},{:.4},{:.4},{:.4},{:.3},{:.1},{:.2},{:.2}",
                st.time / self.day_s,
                relief[*i],
                r[di::TG],
                r[di::TA],
                r[di::H],
                r[di::ZI],
                r[di::TAU],
                r[di::LIFT] * 1e6,
                r[di::TAU_DUST],
                r[di::COSZ],
                r[di::SWG],
                norm(st.v[i * l + l - 1]),
                r[di::LID]
            );
        }
        self.pts_csv.write_all(s.as_bytes())?;
        self.hours += 1;
        for i in 0..model.grid.n_cells() {
            let r = &dg[i * di::N..(i + 1) * di::N];
            if r[di::TAU] > tau_t {
                self.n_exceed[i] += 1.0;
            }
            self.sum_tau[i] += r[di::TAU_DUST];
            self.sum_ta[i] += r[di::TA];
            self.sum_h[i] += r[di::H];
            let a = model.grid.area[i];
            self.sw_sums[0] += a * s0 * r[di::COSZ].max(0.0);
            self.sw_sums[1] += a * r[di::SWG];
            self.sw_sums[2] += a * r[di::SW_AIR];
            if r[di::COSZ] > 0.0 {
                self.sum_tg_day[i] += r[di::TG];
                self.n_day[i] += 1.0;
            } else {
                self.sum_tg_night[i] += r[di::TG];
                self.n_night[i] += 1.0;
            }
            self.day_max_tg[i] = self.day_max_tg[i].max(r[di::TG]);
            self.day_min_tg[i] = self.day_min_tg[i].min(r[di::TG]);
            self.day_max_ta[i] = self.day_max_ta[i].max(r[di::TA]);
            self.day_min_ta[i] = self.day_min_ta[i].min(r[di::TA]);
            self.day_max_zi[i] = self.day_max_zi[i].max(r[di::ZI]);
        }
        Ok(())
    }

    /// End of a model day: daily ranges into the annual sums.
    pub fn day_end(&mut self) {
        for i in 0..self.n_exceed.len() {
            self.sum_range_tg[i] += self.day_max_tg[i] - self.day_min_tg[i];
            self.sum_range_ta[i] += self.day_max_ta[i] - self.day_min_ta[i];
            self.sum_zi_max[i] += self.day_max_zi[i];
            self.day_max_tg[i] = f64::MIN;
            self.day_min_tg[i] = f64::MAX;
            self.day_max_ta[i] = f64::MIN;
            self.day_min_ta[i] = f64::MAX;
            self.day_max_zi[i] = 0.0;
        }
        self.days += 1.0;
    }

    /// Planet-wide record every diagnostic time: dust statistics, dust-sea profile, band winds.
    pub fn diag(&mut self, model: &PeModel, st: &PeState, w: &PeWork, relief: &[f64]) -> std::io::Result<()> {
        use crate::phys::di;
        let gr = &model.grid;
        let n = gr.n_cells();
        let l = model.nl;
        let dg = &w.ph.diag;
        let area_tot: f64 = gr.area.iter().sum();
        let mut taus: Vec<(f64, f64)> = (0..n).map(|i| (dg[i * di::N + di::TAU_DUST], gr.area[i])).collect();
        let tau_mean = taus.iter().map(|(t, a)| t * a).sum::<f64>() / area_tot;
        taus.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let quant = |q: f64| {
            let mut acc = 0.0;
            for (t, a) in &taus {
                acc += a;
                if acc >= q * area_tot {
                    return *t;
                }
            }
            taus.last().unwrap().0
        };
        let (pz0, pdz, pn) = self.prof;
        let (mut lift_area, mut lift_tot) = (0.0, 0.0);
        let mut prof = vec![0.0; pn];
        let mut profw = vec![0.0; pn];
        let (mut sea_s, mut sea_w, mut ab_s, mut ab_w, mut bas_s, mut bas_w, mut lid_s, mut lid_w) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        // Per column: base and top of the dust sea = where the fine dust crosses half the target
        // mixing ratio (linear between full levels); averaged over the columns whose ground is
        // below the sea's nominal base (for the base) or top (for the top).
        let (qt, sbase, stop, sabove) = self.sea.unwrap_or((f64::NAN, f64::NAN, f64::NAN, f64::NAN));
        let qh = 0.5 * qt;
        let (mut cb_s, mut cb_a, mut ct_s, mut ct_a, mut touch_a, mut low_a) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for i in 0..n {
            let a = gr.area[i];
            let lift = dg[i * di::N + di::LIFT];
            if lift > 0.0 {
                lift_area += a;
            }
            lift_tot += lift * a;
            let (zh, zf) = model.column_heights(st, i);
            if self.sea.is_some() {
                let q: Vec<f64> = (0..l).map(|k| st.m[i * l + k] / model.dp(k, st.ps[i])).collect();
                let kb = l - 1;
                let mut base = f64::NAN;
                let mut top = f64::NAN;
                if q[kb] >= qh {
                    base = zh[l];
                }
                for k in (0..kb).rev() {
                    let (qa, qb) = (q[k], q[k + 1]);
                    let cross = |z0: f64, z1: f64| z0 + (qh - qb) / (qa - qb) * (z1 - z0);
                    if base.is_nan() && qb < qh && qa >= qh {
                        base = cross(zf[k + 1], zf[k]);
                    }
                    if qb >= qh && qa < qh {
                        top = cross(zf[k + 1], zf[k]);
                    }
                }
                if zh[l] < sbase {
                    low_a += a;
                    if q[kb] >= qh {
                        touch_a += a;
                    } else if base.is_finite() {
                        cb_s += base * a;
                        cb_a += a;
                    }
                }
                if zh[l] < stop && top.is_finite() {
                    ct_s += top * a;
                    ct_a += a;
                }
            }
            for k in 0..l {
                let lo = zh[k + 1];
                let hi = if k > 0 { zh[k] } else { zh[1] + 2.0 * (zf[0] - zh[1]) };
                let dp = model.dp(k, st.ps[i]);
                let q = st.m[i * l + k] / dp;
                let sp = norm(st.v[i * l + k]);
                for b in 0..pn {
                    let (b0, b1) = (pz0 + b as f64 * pdz, pz0 + (b + 1) as f64 * pdz);
                    let ov = (hi.min(b1) - lo.max(b0)).max(0.0);
                    if ov > 0.0 {
                        prof[b] += q * ov * a;
                        profw[b] += ov * a;
                    }
                }
                if self.sea.is_some() {
                    let ws = (hi.min(stop) - lo.max(sbase)).max(0.0) / (hi - lo) * dp * a;
                    sea_s += sp * ws;
                    sea_w += ws;
                    let wa = (hi.min(sabove) - lo.max(stop)).max(0.0) / (hi - lo) * dp * a;
                    ab_s += sp * wa;
                    ab_w += wa;
                }
            }
            if relief[i] < self.classes[0] {
                bas_s += norm(st.v[i * l + l - 1]) * a;
                bas_w += a;
            }
            if dg[i * di::N + di::LID] != 0.0 {
                lid_s += dg[i * di::N + di::LID] * a;
                lid_w += a;
            }
        }
        for b in 0..pn {
            prof[b] = if profw[b] > 0.0 { prof[b] / profw[b] } else { 0.0 };
        }
        let pmax = prof.iter().cloned().fold(0.0, f64::max);
        let zc = |b: usize| pz0 + (b as f64 + 0.5) * pdz;
        let (base, top) = if pmax > 0.0 && self.sea.is_some() {
            let above: Vec<usize> = (0..pn).filter(|&b| prof[b] >= 0.5 * pmax).collect();
            (zc(*above.first().unwrap()) - 0.5 * pdz, zc(*above.last().unwrap()) + 0.5 * pdz)
        } else {
            (f64::NAN, f64::NAN)
        };
        let d = model.diag(st);
        let day = st.time / self.day_s;
        let bs = sea_s / sea_w.max(1e-30);
        let ba = ab_s / ab_w.max(1e-30);
        let bb = bas_s / bas_w.max(1e-30);
        let cbase = if cb_a > 0.0 { cb_s / cb_a } else { f64::NAN };
        let ctouch = touch_a / low_a.max(1e-30);
        let ctop = if ct_a > 0.0 { ct_s / ct_a } else { f64::NAN };
        let _ = writeln!(
            self.dust_csv,
            "{day:.4},{tau_mean:.4},{:.4},{:.4},{:.3},{:.4},{:.4e},{base:.0},{top:.0},{bs:.3},{ba:.3},{bb:.3},{:.3},{cbase:.0},{ctouch:.4},{ctop:.0},{:.4e},{:.4e},{:.4e},{:.4e},{:.4e},{:.4e}",
            quant(0.5),
            quant(0.9),
            taus.last().unwrap().0,
            lift_area / area_tot,
            lift_tot,
            lid_s / lid_w.max(1e-30),
            d.dust_mass,
            d.dust2_mass,
            st.budget[0],
            st.budget[1],
            st.budget[2],
            st.budget[3]
        );
        if let Some(f) = self.prof_csv.as_mut() {
            let mut s = format!("{day:.4}");
            for p in &prof {
                let _ = write!(s, ",{:.4e}", p);
            }
            s.push('\n');
            f.write_all(s.as_bytes())?;
        }
        self.band_sea.push(bs);
        self.band_above.push(ba);
        self.basin.push(bb);
        self.sea_base.push(base);
        self.sea_top.push(top);
        self.col_base.push(cbase);
        self.col_touch.push(ctouch);
        self.col_top.push(ctop);
        self.tau_mean.push(tau_mean);
        self.lift_frac.push(lift_area / area_tot);
        Ok(())
    }

    /// Annual maps (PNG tables) and a JSON summary (`resume_physique.json`; same keys as the
    /// step-4 `resume_etape4.json`, the altitude classes and bands written in the key names).
    pub fn finish(&self, model: &PeModel, smp: &Sampler, dir: &Path, relief: &[f64]) -> std::io::Result<()> {
        let n = model.grid.n_cells();
        let hrs = self.hours.max(1) as f64;
        let days = self.days.max(1.0);
        let div = |a: &[f64], b: &[f64]| -> Vec<f64> { (0..n).map(|i| if b[i] > 0.0 { a[i] / b[i] } else { 0.0 }).collect() };
        let ones = vec![hrs; n];
        let dd = vec![days; n];
        let exceed = div(&self.n_exceed, &ones);
        let tau = div(&self.sum_tau, &ones);
        let rg = div(&self.sum_range_tg, &dd);
        let ra = div(&self.sum_range_ta, &dd);
        let tday = div(&self.sum_tg_day, &self.n_day);
        let tnight = div(&self.sum_tg_night, &self.n_night);
        let zim = div(&self.sum_zi_max, &dd);
        let meta = |field: &'static str, units: &'static str, desc: &'static str| TableMeta {
            field,
            units,
            description: desc,
            time_s: 0.0,
            time_hours: 0.0,
            planet_day_s: self.day_s,
            grid_level: model.grid.level,
            grid_cells: n,
            radius_m: model.grid.radius,
            fixed_range: None,
        };
        let wr = |name: &'static str, units: &'static str, desc: &'static str, v: &[f64]| io::write_table(&dir.join(format!("annee_{name}.png")), smp.w, smp.h, &smp.scalar(v), &meta(name, units, desc));
        wr("depassement_seuil", "1", "fraction of the hours of the recorded year with the surface stress above the lifting threshold", &exceed)?;
        wr("tau_poussiere", "1", "mean dust optical depth over the recorded year", &tau)?;
        wr("amplitude_t_sol", "K", "mean daily range (max - min) of the ground skin temperature", &rg)?;
        wr("amplitude_t_air", "K", "mean daily range (max - min) of the surface-air temperature", &ra)?;
        wr("t_sol_jour", "K", "mean ground temperature while the sun is up", &tday)?;
        wr("t_sol_nuit", "K", "mean ground temperature while the sun is down", &tnight)?;
        wr("couche_limite_max", "m", "mean daily maximum of the boundary-layer depth", &zim)?;
        // Summary.
        let area = &model.grid.area;
        let at: f64 = area.iter().sum();
        let mean = |v: &[f64]| (0..n).map(|i| v[i] * area[i]).sum::<f64>() / at;
        let mean_if = |v: &[f64], f: &dyn Fn(usize) -> bool| {
            let (mut s, mut a) = (0.0, 0.0);
            for i in 0..n {
                if f(i) {
                    s += v[i] * area[i];
                    a += area[i];
                }
            }
            s / a.max(1e-30)
        };
        let avg = |v: &[f64]| {
            let f: Vec<f64> = v.iter().cloned().filter(|x| x.is_finite()).collect();
            if f.is_empty() {
                f64::NAN
            } else {
                f.iter().sum::<f64>() / f.len() as f64
            }
        };
        let mnx = |v: &[f64]| {
            let f: Vec<f64> = v.iter().cloned().filter(|x| x.is_finite()).collect();
            if f.is_empty() {
                (f64::NAN, f64::NAN)
            } else {
                (f.iter().cloned().fold(f64::MAX, f64::min), f.iter().cloned().fold(f64::MIN, f64::max))
            }
        };
        let [c0, c1, c2] = self.classes;
        let low = |i: usize| relief[i] < c0;
        let mid = |i: usize| relief[i] >= c0 && relief[i] < c1;
        let high = |i: usize| relief[i] >= c2;
        let (k_low, k_mid, k_high) = (format!("bassins_sous_{c0:.0}m"), format!("{c0:.0}_{c1:.0}m"), format!("au_dessus_{c2:.0}m"));
        let classes = |v: &[f64], digits: usize| {
            format!(
                "{{\"planete\": {:.d$}, \"{k_low}\": {:.d$}, \"{k_mid}\": {:.d$}, \"{k_high}\": {:.d$}}}",
                mean(v),
                mean_if(v, &low),
                mean_if(v, &mid),
                mean_if(v, &high),
                d = digits
            )
        };
        let stat = |v: &[f64], digits: usize| {
            let (a, b) = mnx(v);
            format!("{{\"moyenne\": {:.d$}, \"min\": {:.d$}, \"max\": {:.d$}}}", avg(v), a, b, d = digits)
        };
        let (zb, zt, za) = self.sea.map(|x| (x.1, x.2, x.3)).unwrap_or((f64::NAN, f64::NAN, f64::NAN));
        let exc_any = (0..n).filter(|&i| exceed[i] > 0.0).map(|i| area[i]).sum::<f64>() / at;
        let exc_10 = (0..n).filter(|&i| exceed[i] > 0.1).map(|i| area[i]).sum::<f64>() / at;
        let mut j = String::from("{\n");
        let _ = writeln!(j, "  \"hours\": {}, \"days\": {},", self.hours, self.days);
        let _ = writeln!(
            j,
            "  \"t_air_moyenne_K\": {:.2}, \"flux_chaleur_moyen_Wm2\": {:.2}, \"albedo_planetaire\": {:.4}, \"soleil_absorbe_sol_Wm2\": {:.2}, \"soleil_absorbe_poussiere_Wm2\": {:.2},",
            mean(&div(&self.sum_ta, &ones)),
            mean(&div(&self.sum_h, &ones)),
            1.0 - (self.sw_sums[1] + self.sw_sums[2]) / self.sw_sums[0].max(1e-30),
            self.sw_sums[1] / (hrs * at),
            self.sw_sums[2] / (hrs * at)
        );
        let _ = writeln!(
            j,
            "  \"t_sol_moyenne_K\": {:.2}, \"t_sol_jour_K\": {:.2}, \"t_sol_nuit_K\": {:.2},",
            mean(&div(&(0..n).map(|i| self.sum_tg_day[i] + self.sum_tg_night[i]).collect::<Vec<_>>(), &ones)),
            mean(&tday),
            mean(&tnight)
        );
        let _ = writeln!(j, "  \"amplitude_t_sol_K\": {},", classes(&rg, 2));
        let _ = writeln!(j, "  \"amplitude_t_air_K\": {},", classes(&ra, 2));
        let _ = writeln!(
            j,
            "  \"tau_poussiere_moyen\": {:.4}, \"tau_poussiere_moyen_bassins\": {:.4}, \"tau_poussiere_moyen_hauts_plateaux\": {:.4},",
            mean(&tau),
            mean_if(&tau, &low),
            mean_if(&tau, &high)
        );
        let _ = writeln!(
            j,
            "  \"part_heures_seuil_depasse_planete\": {:.5}, \"part_surface_jamais_sous_seuil\": {:.4}, \"part_surface_seuil_depasse_plus_10pc_du_temps\": {:.4},",
            mean(&exceed),
            exc_any,
            exc_10
        );
        let _ = writeln!(j, "  \"part_heures_seuil_depasse\": {},", classes(&exceed, 5));
        let _ = writeln!(j, "  \"part_surface_soulevement_moyenne\": {:.5},", avg(&self.lift_frac));
        let _ = writeln!(j, "  \"mer_base_m\": {}, \"mer_sommet_m\": {},", stat(&self.sea_base, 0), stat(&self.sea_top, 0));
        let _ = writeln!(
            j,
            "  \"mer_base_colonnes_m\": {}, \"part_colonnes_poussiere_au_sol\": {}, \"mer_sommet_colonnes_m\": {},",
            stat(&self.col_base, 0),
            stat(&self.col_touch, 3),
            stat(&self.col_top, 0)
        );
        let _ = writeln!(
            j,
            "  \"vent_mer_{zb:.0}_{zt:.0}_ms\": {:.3}, \"vent_dessus_{zt:.0}_{za:.0}_ms\": {:.3}, \"vent_bas_bassins_ms\": {:.3},",
            avg(&self.band_sea),
            avg(&self.band_above),
            avg(&self.basin)
        );
        let _ = writeln!(j, "  \"couche_limite_max_moyenne_m\": {:.0}\n}}", mean(&zim));
        // An empty statistic (e.g. no column with an elevated base) is written as null.
        std::fs::write(dir.join("resume_physique.json"), j.replace("NaN", "null"))
    }
}

/// One record of the compact climate file `climat.bin` (little-endian f32):
/// time in model days, then zonal means (96 latitude rows) of u, v, T for every layer,
/// then 192 x 96 maps of sea-level pressure (hPa), u, v and T of the lowest layer.
pub fn climate_record(model: &PeModel, st: &PeState, relief: &[f64], smp: &Sampler, day_s: f64) -> Vec<f32> {
    let l = model.nl;
    let fl = layer_fields(model, st);
    let (w, h) = (smp.w, smp.h);
    let mut rec: Vec<f32> = vec![(st.time / day_s) as f32];
    let zonal = |a: &[f32]| -> Vec<f32> { (0..h).map(|r| (a[r * w..(r + 1) * w].iter().map(|&x| x as f64).sum::<f64>() / w as f64) as f32).collect() };
    let mut us = Vec::new();
    let mut vs = Vec::new();
    let mut ts = Vec::new();
    for k in 0..l {
        let (u, v) = smp.east_north(&fl.u[k]);
        us.extend(zonal(&u));
        vs.extend(zonal(&v));
        ts.extend(zonal(&smp.scalar(&fl.t[k])));
    }
    rec.extend(us);
    rec.extend(vs);
    rec.extend(ts);
    let slp: Vec<f64> = sea_level_pressure(model, st, relief).iter().map(|p| p / 100.0).collect();
    rec.extend(smp.scalar(&slp));
    let (u, v) = smp.east_north(&fl.u[l - 1]);
    rec.extend(u);
    rec.extend(v);
    rec.extend(smp.scalar(&fl.t[l - 1]));
    rec
}

pub fn write_climate_header(model: &PeModel, sc: &Scenario, dir: &Path, relief: &[f64], smp: &Sampler) -> std::io::Result<()> {
    let sig: Vec<String> = model.sig_full.iter().map(|s| format!("{s:.6}")).collect();
    let sigh: Vec<String> = model.eta_half.iter().map(|s| format!("{s:.6}")).collect();
    let bh: Vec<String> = model.b_half.iter().map(|s| format!("{s:.6}")).collect();
    let s = &sc.s;
    let json = format!(
        concat!(
            "{{\n  \"format\": \"little-endian float32 records, one per diagnostic time\",\n",
            "  \"record\": [\"time_planet_days\", \"u_zonal[layer][lat]\", \"v_zonal[layer][lat]\", \"t_zonal_K[layer][lat]\", \"pmer_hPa[lat][lon]\", \"u_bas[lat][lon]\", \"v_bas[lat][lon]\", \"t_bas_K[lat][lon]\"],\n",
            "  \"nlat\": {}, \"nlon\": {}, \"nlayers\": {},\n",
            "  \"lat\": \"90 - (row + 0.5) * 180 / nlat\", \"lon\": \"-180 + (col + 0.5) * 360 / nlon\",\n",
            "  \"sigma_full\": [{}], \"sigma_half\": [{}], \"b_half\": [{}], \"hybrid_eta_t\": {}, \"hyper_order\": {}, \"layer_order\": \"top to ground\",\n",
            "  \"sigma_note\": \"sigma_* = p / p0 of a column whose surface pressure is p0 (hybrid levels: p = A + B ps)\",\n",
            "  \"body\": \"{}\", \"planet_day_s\": {}, \"radius_m\": {}, \"gravity\": {}, \"p0_pa\": {},\n",
            "  \"level\": {}, \"dt_y_K\": {}, \"t_max_K\": {}, \"seasons\": {}, \"smooth_passes\": {}, \"relief_scale\": {},\n",
            "  \"spinup_days\": {}, \"record_days\": {}, \"diag_hours\": {}\n}}\n"
        ),
        smp.h,
        smp.w,
        model.nl,
        sig.join(", "),
        sigh.join(", "),
        bh.join(", "),
        model.eta_t,
        model.hyper_order,
        sc.body.name,
        sc.body.day_s,
        model.grid.radius,
        model.c.g,
        model.c.p0,
        model.grid.level,
        sc.dt_y(),
        sc.t_max(),
        s.relaxation.seasons,
        s.relief.smooth_passes,
        s.relief.scale,
        s.run.spinup_days,
        s.run.record_days,
        s.run.diag_every_days * sc.body.day_s / 3600.0
    );
    std::fs::write(dir.join("climat.json"), json)?;
    // Model relief on the same 192 x 96 grid, for the figures.
    let r: Vec<u8> = smp.scalar(relief).iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(dir.join("relief_192x96.f32"), r)
}
