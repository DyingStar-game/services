//! Off-road path planning between buildings: from a village garage to the
//! nearest road (which may lie several hundred metres away), and from the road
//! to the garage at the destination.
//!
//! The obstacles are the ground footprints of the village's solid collision
//! shapes, exported from Godot with its layout (`scene_layouts`) and placed in
//! the planet frame with the replicated village pose. Planning is a grid A* in
//! the tangent plane, obstacles inflated by the vehicle's half width plus a
//! margin, followed by line-of-sight smoothing so the controller gets a few
//! long straight legs instead of a staircase. Terrain itself (slopes, rocks)
//! is not known here: the controller's unstick manoeuvre handles the odd snag.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
};

use crate::{horizon::messages::Vec3, world_data::geo};

/// Free space kept around obstacles: half the truck's width plus a margin.
pub const DEFAULT_CLEARANCE_M: f64 = 2.5;
/// Grid resolution.
pub const DEFAULT_CELL_M: f64 = 1.5;
/// Room around the start→goal box the path may swing out into, widened in
/// these steps while no way round is found.
const MARGINS_M: [f64; 3] = [80.0, 250.0, 600.0];
/// Upper bound on grid cells; the cell size grows past it.
const MAX_CELLS: usize = 1_500_000;

/// A footprint to keep out of: its corners, planet frame (any order around).
pub type Obstacle = Vec<Vec3>;

struct Grid {
    tan: geo::Tangent,
    radius: f64,
    x0: f64,
    y0: f64,
    cell: f64,
    w: usize,
    h: usize,
    blocked: Vec<bool>,
}

impl Grid {
    fn idx(&self, i: usize, j: usize) -> usize {
        j * self.w + i
    }

    fn cell_of(&self, x: f64, y: f64) -> (usize, usize) {
        let i = ((x - self.x0) / self.cell).floor().clamp(0.0, (self.w - 1) as f64) as usize;
        let j = ((y - self.y0) / self.cell).floor().clamp(0.0, (self.h - 1) as f64) as usize;
        (i, j)
    }

    fn centre(&self, i: usize, j: usize) -> (f64, f64) {
        (self.x0 + (i as f64 + 0.5) * self.cell, self.y0 + (j as f64 + 0.5) * self.cell)
    }

    fn free(&self, i: usize, j: usize) -> bool {
        !self.blocked[self.idx(i, j)]
    }

    fn to_planet(&self, x: f64, y: f64) -> Vec3 {
        geo::scale(geo::normalized(self.tan.unproject(x, y)), self.radius)
    }

    /// Nearest free cell to (i, j), breadth first.
    fn nearest_free(&self, i: usize, j: usize) -> Option<(usize, usize)> {
        if self.free(i, j) {
            return Some((i, j));
        }
        let mut seen = vec![false; self.w * self.h];
        let mut q = VecDeque::from([(i, j)]);
        seen[self.idx(i, j)] = true;
        while let Some((a, b)) = q.pop_front() {
            if self.free(a, b) {
                return Some((a, b));
            }
            for (da, db) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (na, nb) = (a as i64 + da, b as i64 + db);
                if na < 0 || nb < 0 || na >= self.w as i64 || nb >= self.h as i64 {
                    continue;
                }
                let k = self.idx(na as usize, nb as usize);
                if !seen[k] {
                    seen[k] = true;
                    q.push_back((na as usize, nb as usize));
                }
            }
        }
        None
    }

    /// Whether the straight segment between two plane points stays in free
    /// cells (sampled every third of a cell).
    fn line_free(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let n = ((len / (self.cell / 3.0)).ceil() as usize).max(1);
        (0..=n).all(|k| {
            let t = k as f64 / n as f64;
            let (i, j) = self.cell_of(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            self.free(i, j)
        })
    }
}

/// Distance from point `p` to the convex polygon `poly` (0 inside).
fn dist_to_polygon(p: (f64, f64), poly: &[(f64, f64)]) -> f64 {
    let n = poly.len();
    let mut inside = true;
    let mut sign = 0.0;
    let mut best = f64::INFINITY;
    for k in 0..n {
        let a = poly[k];
        let b = poly[(k + 1) % n];
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != 0.0 && cross.signum() != sign {
            inside = false;
        }
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l2 = dx * dx + dy * dy;
        let t = if l2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) };
        best = best.min((a.0 + dx * t - p.0).hypot(a.1 + dy * t - p.1));
    }
    if inside && n >= 3 { 0.0 } else { best }
}

#[derive(Copy, Clone, PartialEq)]
struct QItem {
    f: f64,
    k: usize,
}
impl Eq for QItem {}
impl Ord for QItem {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f)
    }
}
impl PartialOrd for QItem {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// A drivable path from `start` to `goal` (planet frame) around `obstacles`,
/// as a few waypoints (planet frame, on the sphere of `start`'s radius),
/// `start` and `goal` included. A start or goal inside an inflated obstacle is
/// moved to the nearest free spot. `None` when no way through exists.
pub fn plan(
    start: Vec3,
    goal: Vec3,
    obstacles: &[Obstacle],
    clearance_m: f64,
    cell_m: f64,
) -> Option<Vec<Vec3>> {
    MARGINS_M
        .iter()
        .find_map(|&m| plan_within(start, goal, obstacles, clearance_m, cell_m, m))
}

fn plan_within(
    start: Vec3,
    goal: Vec3,
    obstacles: &[Obstacle],
    clearance_m: f64,
    cell_m: f64,
    margin_m: f64,
) -> Option<Vec<Vec3>> {
    let radius = geo::length(start);
    let mid = geo::scale(geo::normalized(geo::add(start, goal)), radius);
    let tan = geo::Tangent::at(mid);
    let s = tan.project(start);
    let g = tan.project(goal);

    let polys: Vec<Vec<(f64, f64)>> = obstacles
        .iter()
        .filter(|o| o.len() >= 2)
        .map(|o| o.iter().map(|p| tan.project(geo::scale(geo::normalized(*p), radius))).collect())
        .collect();

    let x0 = s.0.min(g.0) - margin_m;
    let y0 = s.1.min(g.1) - margin_m;
    let x1 = s.0.max(g.0) + margin_m;
    let y1 = s.1.max(g.1) + margin_m;
    let mut cell = cell_m.max(0.25);
    while ((x1 - x0) / cell).ceil() * ((y1 - y0) / cell).ceil() > MAX_CELLS as f64 {
        cell *= 1.5;
    }
    let w = ((x1 - x0) / cell).ceil() as usize;
    let h = ((y1 - y0) / cell).ceil() as usize;
    let mut grid = Grid { tan, radius, x0, y0, cell, w, h, blocked: vec![false; w * h] };

    // Rasterise the inflated footprints.
    for poly in &polys {
        let (mut px0, mut py0, mut px1, mut py1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in poly {
            px0 = px0.min(p.0);
            py0 = py0.min(p.1);
            px1 = px1.max(p.0);
            py1 = py1.max(p.1);
        }
        if px1 + clearance_m < x0 || px0 - clearance_m > x1 || py1 + clearance_m < y0 || py0 - clearance_m > y1 {
            continue;
        }
        let (i0, j0) = grid.cell_of(px0 - clearance_m, py0 - clearance_m);
        let (i1, j1) = grid.cell_of(px1 + clearance_m, py1 + clearance_m);
        for j in j0..=j1 {
            for i in i0..=i1 {
                if dist_to_polygon(grid.centre(i, j), poly) <= clearance_m {
                    let k = grid.idx(i, j);
                    grid.blocked[k] = true;
                }
            }
        }
    }

    let (si, sj) = grid.cell_of(s.0, s.1);
    let (gi, gj) = grid.cell_of(g.0, g.1);
    let (si, sj) = grid.nearest_free(si, sj)?;
    let (gi, gj) = grid.nearest_free(gi, gj)?;
    let s_free = if (si, sj) == grid.cell_of(s.0, s.1) { s } else { grid.centre(si, sj) };
    let g_free = if (gi, gj) == grid.cell_of(g.0, g.1) { g } else { grid.centre(gi, gj) };

    let pts: Vec<(f64, f64)> = if grid.line_free(s_free, g_free) {
        vec![s_free, g_free]
    } else {
        let cells = astar(&grid, (si, sj), (gi, gj))?;
        let mut raw: Vec<(f64, f64)> = cells.iter().map(|&(i, j)| grid.centre(i, j)).collect();
        raw[0] = s_free;
        *raw.last_mut().unwrap() = g_free;
        smooth(&grid, &raw)
    };

    let mut out = Vec::with_capacity(pts.len() + 2);
    if s_free != s {
        out.push(start);
    }
    out.extend(pts.iter().map(|&(x, y)| grid.to_planet(x, y)));
    if g_free != g {
        out.push(goal);
    }
    Some(out)
}

fn astar(grid: &Grid, s: (usize, usize), g: (usize, usize)) -> Option<Vec<(usize, usize)>> {
    let n = grid.w * grid.h;
    let mut cost = vec![f64::INFINITY; n];
    let mut prev = vec![usize::MAX; n];
    let hfn = |i: usize, j: usize| {
        let dx = (i as f64 - g.0 as f64).abs();
        let dy = (j as f64 - g.1 as f64).abs();
        // Octile distance, in cells.
        dx.max(dy) + (std::f64::consts::SQRT_2 - 1.0) * dx.min(dy)
    };
    let sk = grid.idx(s.0, s.1);
    let gk = grid.idx(g.0, g.1);
    cost[sk] = 0.0;
    let mut open = BinaryHeap::from([QItem { f: hfn(s.0, s.1), k: sk }]);
    while let Some(QItem { f, k }) = open.pop() {
        if k == gk {
            break;
        }
        let (i, j) = (k % grid.w, k / grid.w);
        if f - hfn(i, j) > cost[k] + 1e-9 {
            continue;
        }
        for (di, dj) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let (ni, nj) = (i as i64 + di, j as i64 + dj);
            if ni < 0 || nj < 0 || ni >= grid.w as i64 || nj >= grid.h as i64 {
                continue;
            }
            let (ni, nj) = (ni as usize, nj as usize);
            if !grid.free(ni, nj) {
                continue;
            }
            // No corner cutting between two blocked cells.
            if di != 0 && dj != 0 && (!grid.free(ni, j) || !grid.free(i, nj)) {
                continue;
            }
            let nk = grid.idx(ni, nj);
            let c = cost[k] + if di != 0 && dj != 0 { std::f64::consts::SQRT_2 } else { 1.0 };
            if c < cost[nk] {
                cost[nk] = c;
                prev[nk] = k;
                open.push(QItem { f: c + hfn(ni, nj), k: nk });
            }
        }
    }
    if !cost[gk].is_finite() {
        return None;
    }
    let mut path = vec![g];
    let mut k = gk;
    while k != sk {
        k = prev[k];
        path.push((k % grid.w, k / grid.w));
    }
    path.reverse();
    Some(path)
}

/// Keep only the waypoints needed to stay in free space: from each kept
/// point, jump to the furthest one still in line of sight.
fn smooth(grid: &Grid, raw: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![raw[0]];
    let mut i = 0;
    while i < raw.len() - 1 {
        let mut j = raw.len() - 1;
        while j > i + 1 && !grid.line_free(raw[i], raw[j]) {
            j -= 1;
        }
        out.push(raw[j]);
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 6_356_000.0;

    /// Planet-frame point `e` m east and `n` m north of (lon 10, lat 20).
    fn at(e: f64, n: f64) -> Vec3 {
        geo::Tangent::at(geo::scale(geo::lonlat_to_unit(10.0, 20.0), R)).unproject(e, n)
    }

    fn rect(e0: f64, n0: f64, e1: f64, n1: f64) -> Obstacle {
        vec![at(e0, n0), at(e1, n0), at(e1, n1), at(e0, n1)]
    }

    fn local(p: Vec3) -> (f64, f64) {
        geo::Tangent::at(geo::scale(geo::lonlat_to_unit(10.0, 20.0), R)).project(p)
    }

    #[test]
    fn open_ground_is_one_straight_leg() {
        let path = plan(at(0.0, 0.0), at(600.0, 300.0), &[], DEFAULT_CLEARANCE_M, DEFAULT_CELL_M).unwrap();
        assert_eq!(path.len(), 2);
    }

    #[test]
    fn goes_round_a_wall_through_its_gap() {
        // A wall north-south at e=50, from n=-200 to n=200, with a 12 m gap at n=100.
        let obstacles = vec![rect(49.0, -200.0, 51.0, 94.0), rect(49.0, 106.0, 51.0, 200.0)];
        let path = plan(at(0.0, 0.0), at(100.0, 0.0), &obstacles, DEFAULT_CLEARANCE_M, DEFAULT_CELL_M)
            .expect("there is a gap");
        assert!(path.len() > 2, "must bend");
        // Every leg stays clear of the wall's inflated footprint.
        let pts: Vec<(f64, f64)> = path.iter().map(|p| local(*p)).collect();
        let near_wall = pts.windows(2).any(|w| {
            (0..=100).any(|k| {
                let t = k as f64 / 100.0;
                let (x, y) = (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
                (x - 50.0).abs() < 2.0 && !(94.0..=106.0).contains(&y)
            })
        });
        assert!(!near_wall, "path crosses the wall: {pts:?}");
        let crossing = pts.iter().any(|p| (p.1 - 100.0).abs() < 10.0);
        assert!(crossing, "path does not use the gap: {pts:?}");
    }

    #[test]
    fn start_inside_a_footprint_is_moved_out() {
        // Truck parked right against a container.
        let obstacles = vec![rect(-1.0, -1.0, 1.0, 6.0)];
        let path = plan(at(0.5, 0.0), at(0.0, -300.0), &obstacles, DEFAULT_CLEARANCE_M, DEFAULT_CELL_M);
        assert!(path.is_some());
    }

    #[test]
    fn walled_in_has_no_path() {
        let obstacles = vec![
            rect(-30.0, -30.0, 30.0, -28.0),
            rect(-30.0, 28.0, 30.0, 30.0),
            rect(-30.0, -30.0, -28.0, 30.0),
            rect(28.0, -30.0, 30.0, 30.0),
        ];
        assert!(plan(at(0.0, 0.0), at(500.0, 0.0), &obstacles, DEFAULT_CLEARANCE_M, DEFAULT_CELL_M).is_none());
    }

    /// The shipped village layouts: from outside each garage door, a truck
    /// finds its way round the buildings to a road 700 m away in any
    /// direction — and every leg of the path stays clear of every footprint.
    #[test]
    fn the_real_villages_can_be_driven_out_of_in_every_direction() {
        use crate::scene_layouts::{GARAGE_SCENE, SceneLayouts};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/scene_layouts");
        let layouts = SceneLayouts::load(&dir);
        let garage = layouts.get(GARAGE_SCENE).expect("garage layout");
        let spawn = garage.kiosks[0].spawn.clone().expect("spawn");
        let outside_local =
            geo::add(spawn.position(), geo::scale(spawn.forward(), 15.0));
        // Layout frame on the tangent plane: x = east, z = south (Godot -Z = north).
        let to_plane = |v: Vec3| at(v.x, -v.z);
        for scene in [
            "scenes/_universe/structures/urban/villages/ares_village_mining.tscn",
            "scenes/_universe/structures/urban/cities/ares_city_factory.tscn",
        ] {
            let village = layouts.get(scene).expect("village layout");
            assert!(!village.obstacles.is_empty() && !village.garages.is_empty());
            let obstacles: Vec<Obstacle> = village
                .obstacles
                .iter()
                .map(|o| o.corner_points().map(to_plane).collect())
                .collect();
            for g in &village.garages {
                let start = to_plane(g.to_village(outside_local));
                for k in 0..8 {
                    let a = k as f64 * std::f64::consts::FRAC_PI_4;
                    let (sx, sy) = local(start);
                    let goal = at(sx + 700.0 * a.cos(), sy + 700.0 * a.sin());
                    let path = plan(start, goal, &obstacles, 2.0, 1.0)
                        .unwrap_or_else(|| panic!("{scene} {}: no way out toward {k}", g.name));
                    let pts: Vec<(f64, f64)> = path.iter().map(|p| local(*p)).collect();
                    let polys: Vec<Vec<(f64, f64)>> =
                        obstacles.iter().map(|o| o.iter().map(|p| local(*p)).collect()).collect();
                    for w in pts.windows(2) {
                        for t in 0..=200 {
                            let t = t as f64 / 200.0;
                            let q = (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
                            for poly in &polys {
                                assert!(
                                    dist_to_polygon(q, poly) > 1.0,
                                    "{scene} {}: leg {w:?} grazes a building toward {k}",
                                    g.name
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn point_polygon_distance() {
        let sq = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        assert_eq!(dist_to_polygon((1.0, 1.0), &sq), 0.0);
        assert!((dist_to_polygon((5.0, 1.0), &sq) - 3.0).abs() < 1e-9);
    }
}
