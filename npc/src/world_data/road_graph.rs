//! Road network of a planet as a routable graph, built from the QGIS road
//! export (`DyingStar/assets/qgis/export/<planet>_roads_buffered.json`, copied
//! under `WORLD_DATA_DIR/<planet>/roads.json`).
//!
//! The export has no topology: each feature is a bare lon/lat centerline. The
//! graph is inferred here:
//!
//! - every centerline vertex becomes a node; a vertex lying within `snap_m` of
//!   a node of ANOTHER road is merged into it (that is how intersections are
//!   drawn in QGIS: shared, or nearly shared, vertices);
//! - a dead end lying within `snap_m` of the middle of another road's segment
//!   splits that segment (T-junction drawn without a shared vertex);
//! - consecutive vertices are joined by an edge. Between two vertices the road
//!   is LINEAR IN LON/LAT, exactly like the game's road exporter lays the
//!   ribbon (`modifier_geom.partition_polyline`), so routes are densified the
//!   same way and stay on the rendered asphalt.
//!
//! Railways are skipped. The graph is small (a few thousand nodes on
//! tarsis_3), so routing is a plain Dijkstra.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

use serde::Deserialize;
use serde_json::{Value, json};

use super::geo;
use crate::horizon::messages::Vec3;

/// Default distance under which vertices of two roads are one intersection.
pub const DEFAULT_SNAP_M: f64 = 15.0;

/// Spacing of the points of a route handed to the driving controller.
pub const ROUTE_STEP_M: f64 = 10.0;

/// Speed assumed for a road whose export carries no `speed_limit`.
const DEFAULT_SPEED_KMH: f64 = 50.0;

#[derive(Debug, Deserialize)]
struct FeatureCollection {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    #[serde(default)]
    properties: Value,
}

/// One road, as read from the export.
#[derive(Debug, Clone)]
pub struct RoadDef {
    pub road_type: String,
    pub speed_kmh: f64,
    /// Total paved width, metres (the game's road ribbon).
    pub width_m: f64,
    /// Traffic lanes, both directions together.
    pub lanes: u8,
    pub centerline: Vec<(f64, f64)>,
}

/// Width and lanes of a road type when the export does not say
/// (`road_terrain.gd`: 3.5 m lanes, highways 4 lanes with a 0.5 m median).
pub fn default_profile(road_type: &str) -> (f64, u8) {
    match road_type {
        "highway" => (14.5, 4),
        "path" => (2.0, 1),
        "trail" => (1.0, 1),
        _ => (6.0, 2),
    }
}

/// What a stretch of a route is like: its speed limit, and the road it is on
/// (`half_width_m` 0 and `lanes` 0 off the road).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Stretch {
    pub speed_kmh: f64,
    pub half_width_m: f64,
    pub lanes: u8,
}

impl Stretch {
    pub fn offroad(speed_kmh: f64) -> Self {
        Self { speed_kmh, half_width_m: 0.0, lanes: 0 }
    }
}

impl RoadDef {
    fn stretch(&self, speed_kmh: f64) -> Stretch {
        Stretch { speed_kmh, half_width_m: self.width_m / 2.0, lanes: self.lanes }
    }

    /// Relative cost of driving one metre of this road: the router prefers the
    /// big roads, and only uses trails when nothing else connects.
    fn cost_factor(&self) -> f64 {
        match self.road_type.as_str() {
            "highway" => 0.85,
            "road" => 1.0,
            _ => 1.6,
        }
    }
}

/// Parse the roads GeoJSON. Railways and degenerate lines are dropped, and the
/// duplicates the exporter writes (one feature per buffered polygon part, each
/// carrying the same centerline) are collapsed.
pub fn parse_roads(text: &str) -> Result<Vec<RoadDef>, serde_json::Error> {
    let fc: FeatureCollection = serde_json::from_str(text)?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for f in fc.features {
        let p = &f.properties;
        let road_type = p
            .get("road_type")
            .and_then(Value::as_str)
            .unwrap_or("road")
            .to_ascii_lowercase();
        if road_type == "railway" {
            continue;
        }
        let Some(cl) = p.get("centerline").and_then(Value::as_array) else {
            continue;
        };
        let centerline: Vec<(f64, f64)> = cl
            .iter()
            .filter_map(|pt| {
                let a = pt.as_array()?;
                Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
            })
            .collect();
        if centerline.len() < 2 {
            continue;
        }
        let key = format!("{:?}", centerline);
        if !seen.insert(key) {
            continue;
        }
        let speed_kmh = p
            .get("speed_limit")
            .and_then(Value::as_f64)
            .filter(|s| *s > 0.0)
            .unwrap_or(DEFAULT_SPEED_KMH);
        let (default_width, default_lanes) = default_profile(&road_type);
        let width_m = p.get("width").and_then(Value::as_f64).filter(|w| *w > 0.0).unwrap_or(default_width);
        let lanes = p
            .get("lanes")
            .and_then(Value::as_u64)
            .filter(|l| *l > 0)
            .map(|l| l.min(16) as u8)
            .unwrap_or(default_lanes);
        out.push(RoadDef { road_type, speed_kmh, width_m, lanes, centerline });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct Node {
    lonlat: (f64, f64),
    unit: Vec3,
}

#[derive(Debug, Clone)]
struct Edge {
    a: usize,
    b: usize,
    length_m: f64,
    cost: f64,
    speed_kmh: f64,
    road: usize,
}

/// A point on the network: `t` ∈ [0, 1] along edge `edge` (from `a` to `b`),
/// plus how far the queried point lies from it.
#[derive(Debug, Clone, Copy)]
pub struct Location {
    pub edge: usize,
    pub t: f64,
    pub lonlat: (f64, f64),
    pub offset_m: f64,
}

/// One point of a route: where (lon/lat and planet-frame unit direction), how
/// far along the route, the speed limit there, and the road it is on
/// (`half_width_m` and `lanes` are 0 off the road).
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct RoutePoint {
    pub lon: f64,
    pub lat: f64,
    #[serde(skip)]
    pub unit: Vec3,
    pub s_m: f64,
    pub speed_kmh: f64,
    pub half_width_m: f64,
    pub lanes: u8,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Route {
    pub points: Vec<RoutePoint>,
    pub length_m: f64,
}

impl Route {
    /// The route as a GeoJSON LineString feature, to eyeball it in QGIS over
    /// the road layer.
    pub fn to_geojson(&self, props: Value) -> Value {
        json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": props,
                "geometry": {
                    "type": "LineString",
                    "coordinates": self.points.iter().map(|p| [p.lon, p.lat]).collect::<Vec<_>>(),
                }
            }]
        })
    }
}

pub struct RoadGraph {
    radius_m: f64,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    adj: Vec<Vec<usize>>,
    roads: Vec<RoadDef>,
}

/// Spatial hash over unit vectors, cell ≈ `snap_m` on the surface.
struct Grid {
    cell: f64,
    map: HashMap<(i64, i64, i64), Vec<usize>>,
}

impl Grid {
    fn new(cell_rad: f64) -> Self {
        Self { cell: cell_rad, map: HashMap::new() }
    }
    fn key(&self, u: Vec3) -> (i64, i64, i64) {
        (
            (u.x / self.cell).floor() as i64,
            (u.y / self.cell).floor() as i64,
            (u.z / self.cell).floor() as i64,
        )
    }
    fn insert(&mut self, u: Vec3, id: usize) {
        self.map.entry(self.key(u)).or_default().push(id);
    }
    fn near(&self, u: Vec3) -> impl Iterator<Item = usize> + '_ {
        let (x, y, z) = self.key(u);
        (-1..=1).flat_map(move |dx| {
            (-1..=1).flat_map(move |dy| {
                (-1..=1).flat_map(move |dz| {
                    self.map
                        .get(&(x + dx, y + dy, z + dz))
                        .into_iter()
                        .flatten()
                        .copied()
                })
            })
        })
    }
}

#[derive(Copy, Clone, PartialEq)]
struct QItem {
    cost: f64,
    node: usize,
}
impl Eq for QItem {}
impl Ord for QItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost)
    }
}
impl PartialOrd for QItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl RoadGraph {
    pub fn build(roads: Vec<RoadDef>, radius_m: f64, snap_m: f64) -> Self {
        let mut g = RoadGraph {
            radius_m,
            nodes: Vec::new(),
            edges: Vec::new(),
            adj: Vec::new(),
            roads,
        };
        let mut grid = Grid::new((snap_m / radius_m).max(1e-12));
        // Which road first created each node: a vertex only merges into a
        // node of ANOTHER road (or an exact duplicate), so a road's own closely
        // spaced vertices never collapse into one.
        let mut owner: Vec<usize> = Vec::new();

        for r in 0..g.roads.len() {
            let mut prev: Option<usize> = None;
            let cl = g.roads[r].centerline.clone();
            for &(lon, lat) in &cl {
                let u = geo::lonlat_to_unit(lon, lat);
                let found = grid
                    .near(u)
                    .map(|id| (id, geo::arc_m(u, g.nodes[id].unit, radius_m)))
                    .filter(|&(id, d)| d <= snap_m && (owner[id] != r || d < 0.01))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(id, _)| id);
                let id = found.unwrap_or_else(|| {
                    let id = g.nodes.len();
                    g.nodes.push(Node { lonlat: (lon, lat), unit: u });
                    g.adj.push(Vec::new());
                    owner.push(r);
                    grid.insert(u, id);
                    id
                });
                if let Some(p) = prev {
                    if p != id {
                        g.add_edge(p, id, r);
                    }
                }
                prev = Some(id);
            }
        }

        g.join_t_junctions(snap_m);
        g
    }

    fn add_edge(&mut self, a: usize, b: usize, road: usize) -> usize {
        let length_m = self.lonlat_length(self.nodes[a].lonlat, self.nodes[b].lonlat);
        let rd = &self.roads[road];
        let e = Edge {
            a,
            b,
            length_m,
            cost: length_m * rd.cost_factor(),
            speed_kmh: rd.speed_kmh,
            road,
        };
        let id = self.edges.len();
        self.edges.push(e);
        self.adj[a].push(id);
        self.adj[b].push(id);
        id
    }

    /// Length of the lon/lat-linear segment between two points, integrated in
    /// ~100 m steps (a straight line in degrees is not a great circle).
    fn lonlat_length(&self, a: (f64, f64), b: (f64, f64)) -> f64 {
        let chord = geo::arc_m(
            geo::lonlat_to_unit(a.0, a.1),
            geo::lonlat_to_unit(b.0, b.1),
            self.radius_m,
        );
        let n = ((chord / 100.0).ceil() as usize).clamp(1, 10_000);
        let mut len = 0.0;
        let mut prev = geo::lonlat_to_unit(a.0, a.1);
        for i in 1..=n {
            let (lo, la) = geo::lerp_lonlat(a, b, i as f64 / n as f64);
            let u = geo::lonlat_to_unit(lo, la);
            len += geo::arc_m(prev, u, self.radius_m);
            prev = u;
        }
        len
    }

    /// Connect each dead end to the road it touches mid-segment, by splitting
    /// that segment at the touching point.
    fn join_t_junctions(&mut self, snap_m: f64) {
        let dead_ends: Vec<usize> = (0..self.nodes.len())
            .filter(|&n| self.adj[n].len() == 1)
            .collect();
        for n in dead_ends {
            let own_road = self.edges[self.adj[n][0]].road;
            let ll = self.nodes[n].lonlat;
            let Some(loc) = self.locate_filtered(ll, |e| e.road != own_road && e.a != n && e.b != n)
            else {
                continue;
            };
            if loc.offset_m > snap_m {
                continue;
            }
            let e = self.edges[loc.edge].clone();
            // Touching (nearly) at an existing vertex: just link to it.
            let target = if loc.t * e.length_m <= snap_m {
                e.a
            } else if (1.0 - loc.t) * e.length_m <= snap_m {
                e.b
            } else {
                self.split_edge(loc.edge, loc.t)
            };
            if target != n && !self.adj[n].iter().any(|&x| self.edges[x].a == target || self.edges[x].b == target) {
                self.add_edge(n, target, own_road);
            }
        }
    }

    /// Insert a node at `t` along edge `e`, replacing it by its two halves.
    /// Returns the new node.
    fn split_edge(&mut self, e: usize, t: f64) -> usize {
        let old = self.edges[e].clone();
        let lonlat = geo::lerp_lonlat(self.nodes[old.a].lonlat, self.nodes[old.b].lonlat, t);
        let id = self.nodes.len();
        self.nodes.push(Node { lonlat, unit: geo::lonlat_to_unit(lonlat.0, lonlat.1) });
        self.adj.push(Vec::new());
        // Re-point the old edge at the first half; add the second half.
        let first_len = self.lonlat_length(self.nodes[old.a].lonlat, lonlat);
        let factor = old.cost / old.length_m.max(1e-9);
        self.adj[old.b].retain(|&x| x != e);
        self.edges[e].b = id;
        self.edges[e].length_m = first_len;
        self.edges[e].cost = first_len * factor;
        self.adj[id].push(e);
        self.add_edge(id, old.b, old.road);
        id
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn radius_m(&self) -> f64 {
        self.radius_m
    }

    /// Nearest point of the network to `pos` (planet frame; only its direction
    /// matters).
    pub fn locate(&self, pos: Vec3) -> Option<Location> {
        self.locate_filtered(geo::unit_to_lonlat(pos), |_| true)
    }

    fn locate_filtered(&self, ll: (f64, f64), keep: impl Fn(&Edge) -> bool) -> Option<Location> {
        // Work in an equirectangular plane centred on the query: a segment
        // linear in lon/lat is a straight line there, so the projection is
        // exact in the roads' own parameterisation.
        let m_per_deg = self.radius_m * std::f64::consts::PI / 180.0;
        let kx = ll.1.to_radians().cos() * m_per_deg;
        let to_xy = |p: (f64, f64)| (geo::lon_delta(ll.0, p.0) * kx, (p.1 - ll.1) * m_per_deg);
        let q = geo::lonlat_to_unit(ll.0, ll.1);
        let mut best: Option<Location> = None;
        for (i, e) in self.edges.iter().enumerate() {
            if !keep(e) {
                continue;
            }
            // Quick reject: the segment cannot come closer than its nearest
            // endpoint minus its length.
            let da = geo::arc_m(q, self.nodes[e.a].unit, self.radius_m);
            if let Some(b) = &best {
                if da - e.length_m > b.offset_m {
                    continue;
                }
            }
            let (ax, ay) = to_xy(self.nodes[e.a].lonlat);
            let (bx, by) = to_xy(self.nodes[e.b].lonlat);
            let (dx, dy) = (bx - ax, by - ay);
            let l2 = dx * dx + dy * dy;
            let t = if l2 == 0.0 { 0.0 } else { (-(ax * dx + ay * dy) / l2).clamp(0.0, 1.0) };
            let lonlat = geo::lerp_lonlat(self.nodes[e.a].lonlat, self.nodes[e.b].lonlat, t);
            let offset_m = geo::arc_m(q, geo::lonlat_to_unit(lonlat.0, lonlat.1), self.radius_m);
            if best.as_ref().is_none_or(|b| offset_m < b.offset_m) {
                best = Some(Location { edge: i, t, lonlat, offset_m });
            }
        }
        best
    }

    /// Dijkstra from a location: (cost to every node, predecessor edge).
    fn dijkstra(&self, from: &Location) -> (Vec<f64>, Vec<Option<usize>>) {
        let n = self.nodes.len();
        let mut dist = vec![f64::INFINITY; n];
        let mut pred = vec![None; n];
        let mut heap = BinaryHeap::new();
        let e = &self.edges[from.edge];
        for (node, c) in [(e.a, from.t * e.cost), (e.b, (1.0 - from.t) * e.cost)] {
            if c < dist[node] {
                dist[node] = c;
                heap.push(QItem { cost: c, node });
            }
        }
        while let Some(QItem { cost, node }) = heap.pop() {
            if cost > dist[node] {
                continue;
            }
            for &ei in &self.adj[node] {
                let ed = &self.edges[ei];
                let other = if ed.a == node { ed.b } else { ed.a };
                let c = cost + ed.cost;
                if c < dist[other] {
                    dist[other] = c;
                    pred[other] = Some(ei);
                    heap.push(QItem { cost: c, node: other });
                }
            }
        }
        (dist, pred)
    }

    /// Cheapest cost from `from` to `to` given a Dijkstra run from `from`, and
    /// through which end of `to`'s edge it arrives (`None` = same edge,
    /// straight along it).
    fn arrival(&self, from: &Location, dist: &[f64], to: &Location) -> (f64, Option<usize>) {
        let e = &self.edges[to.edge];
        let via_a = dist[e.a] + to.t * e.cost;
        let via_b = dist[e.b] + (1.0 - to.t) * e.cost;
        let mut best = if via_a <= via_b { (via_a, Some(e.a)) } else { (via_b, Some(e.b)) };
        if from.edge == to.edge {
            let direct = (from.t - to.t).abs() * e.cost;
            if direct <= best.0 {
                best = (direct, None);
            }
        }
        best
    }

    /// Road cost (≈ metres, weighted by road type) from one location to every
    /// other in `targets`. `INFINITY` where unreachable.
    pub fn costs_to(&self, from: &Location, targets: &[Location]) -> Vec<f64> {
        let (dist, _) = self.dijkstra(from);
        targets.iter().map(|t| self.arrival(from, &dist, t).0).collect()
    }

    /// Cheapest route between two locations, densified every
    /// [`ROUTE_STEP_M`]. `None` when they are not connected.
    pub fn route(&self, from: &Location, to: &Location) -> Option<Route> {
        Some(self.densify(&self.legs(from, to)?))
    }

    /// Route from an arbitrary point to another (planet frame), by the road:
    /// straight off-road to the nearest road point, along the network, then
    /// straight off-road to `to`. The off-road stretches are driven at
    /// `offroad_kmh`. `None` when the two road points are not connected.
    pub fn route_points(&self, from: Vec3, to: Vec3, offroad_kmh: f64) -> Option<Route> {
        let a = self.locate(from)?;
        let b = self.locate(to)?;
        let mut legs = self.legs(&a, &b)?;
        legs.insert(0, (geo::unit_to_lonlat(from), Stretch::offroad(offroad_kmh)));
        if let Some(l) = legs.get_mut(1) {
            // The stretch reaching the road is still off-road.
            l.1 = Stretch::offroad(offroad_kmh);
        }
        legs.push((geo::unit_to_lonlat(to), Stretch::offroad(offroad_kmh)));
        Some(self.densify(&legs))
    }

    /// Road waypoints from the road point nearest `from` to the one nearest
    /// `to` (planet frame), as (lon/lat, speed limit of the stretch leading
    /// to it) — to be joined with off-road stretches and [`densify_legs`].
    ///
    /// [`densify_legs`]: Self::densify_legs
    pub fn road_legs(&self, from: Vec3, to: Vec3) -> Option<Vec<((f64, f64), Stretch)>> {
        let a = self.locate(from)?;
        let b = self.locate(to)?;
        self.legs(&a, &b)
    }

    /// Waypoints of the cheapest path between two locations, as (lon/lat,
    /// speed limit of the stretch leading to it).
    fn legs(&self, from: &Location, to: &Location) -> Option<Vec<((f64, f64), Stretch)>> {
        let (dist, pred) = self.dijkstra(from);
        let (cost, via) = self.arrival(from, &dist, to);
        if !cost.is_finite() {
            return None;
        }
        // Waypoints as (lonlat, speed of the stretch leading to it).
        let mut legs: Vec<((f64, f64), Stretch)> = Vec::new();
        let to_edge = &self.edges[to.edge];
        let to_stretch = self.stretch(to_edge);
        match via {
            None => {
                legs.push((from.lonlat, to_stretch));
                legs.push((to.lonlat, to_stretch));
            }
            Some(end) => {
                // Walk predecessors back from the end node to a start-edge node.
                let mut chain = vec![end];
                let mut cur = end;
                let from_edge = &self.edges[from.edge];
                while let Some(ei) = pred[cur] {
                    let ed = &self.edges[ei];
                    cur = if ed.a == cur { ed.b } else { ed.a };
                    chain.push(cur);
                }
                debug_assert!(cur == from_edge.a || cur == from_edge.b);
                chain.reverse();
                let from_stretch = self.stretch(from_edge);
                legs.push((from.lonlat, from_stretch));
                let mut prev = chain[0];
                legs.push((self.nodes[prev].lonlat, from_stretch));
                for &n in &chain[1..] {
                    let stretch = self.adj[prev]
                        .iter()
                        .map(|&ei| &self.edges[ei])
                        .find(|ed| (ed.a == prev && ed.b == n) || (ed.b == prev && ed.a == n))
                        .map(|ed| self.stretch(ed))
                        .unwrap_or_else(|| self.roads[from_edge.road].stretch(DEFAULT_SPEED_KMH));
                    legs.push((self.nodes[n].lonlat, stretch));
                    prev = n;
                }
                legs.push((to.lonlat, to_stretch));
            }
        }
        Some(legs)
    }

    /// Turn waypoints into a polyline with a point at least every
    /// [`ROUTE_STEP_M`], interpolated linearly in lon/lat like the ribbon.
    pub fn densify_legs(&self, legs: &[((f64, f64), Stretch)]) -> Route {
        self.densify(legs)
    }

    /// The stretch of road an edge belongs to.
    fn stretch(&self, e: &Edge) -> Stretch {
        self.roads[e.road].stretch(e.speed_kmh)
    }

    fn densify(&self, legs: &[((f64, f64), Stretch)]) -> Route {
        let mut points: Vec<RoutePoint> = Vec::new();
        let mut s = 0.0;
        let mut push = |ll: (f64, f64), st: Stretch, points: &mut Vec<RoutePoint>| {
            let unit = geo::lonlat_to_unit(ll.0, ll.1);
            if let Some(last) = points.last() {
                let d = geo::arc_m(last.unit, unit, self.radius_m);
                if d < 0.05 {
                    return;
                }
                s += d;
            }
            points.push(RoutePoint {
                lon: ll.0,
                lat: ll.1,
                unit,
                s_m: s,
                speed_kmh: st.speed_kmh,
                half_width_m: st.half_width_m,
                lanes: st.lanes,
            });
        };
        if let Some(&(first, st)) = legs.first() {
            push(first, st, &mut points);
        }
        for w in legs.windows(2) {
            let ((a, _), (b, speed)) = (w[0], w[1]);
            let chord = geo::arc_m(
                geo::lonlat_to_unit(a.0, a.1),
                geo::lonlat_to_unit(b.0, b.1),
                self.radius_m,
            );
            let n = ((chord / ROUTE_STEP_M).ceil() as usize).max(1);
            for i in 1..=n {
                push(geo::lerp_lonlat(a, b, i as f64 / n as f64), speed, &mut points);
            }
        }
        let length_m = points.last().map(|p| p.s_m).unwrap_or(0.0);
        Route { points, length_m }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 6_356_000.0;

    /// Degrees of latitude for `m` metres on the test planet.
    fn deg(m: f64) -> f64 {
        m / (R * std::f64::consts::PI / 180.0)
    }

    fn road(t: &str, pts: &[(f64, f64)]) -> RoadDef {
        let (width_m, lanes) = default_profile(t);
        RoadDef { road_type: t.into(), speed_kmh: 50.0, width_m, lanes, centerline: pts.to_vec() }
    }

    #[test]
    fn parse_skips_railways_and_duplicates() {
        let text = json!({
            "type": "FeatureCollection",
            "features": [
                {"properties": {"road_type": "road", "speed_limit": 70, "centerline": [[0.0, 0.0], [0.1, 0.0]]}},
                {"properties": {"road_type": "road", "speed_limit": 70, "centerline": [[0.0, 0.0], [0.1, 0.0]]}},
                {"properties": {"road_type": "railway", "centerline": [[0.0, 0.0], [0.0, 1.0]]}},
                {"properties": {"road_type": "highway", "centerline": [[1.0, 0.0]]}}
            ]
        })
        .to_string();
        let roads = parse_roads(&text).unwrap();
        assert_eq!(roads.len(), 1);
        assert_eq!(roads[0].speed_kmh, 70.0);
    }

    #[test]
    fn shared_and_nearly_shared_vertices_become_intersections() {
        // A horizontal road and a vertical one whose vertex sits 5 m off the
        // first road's middle vertex.
        let h = road("road", &[(0.0, 0.0), (deg(500.0), 0.0), (deg(1000.0), 0.0)]);
        let v = road("road", &[(deg(500.0), deg(5.0)), (deg(500.0), deg(800.0))]);
        let g = RoadGraph::build(vec![h, v], R, DEFAULT_SNAP_M);
        assert_eq!(g.node_count(), 4, "the 5 m-off vertex must merge");
        assert_eq!(g.edge_count(), 3);
    }

    #[test]
    fn a_roads_own_close_vertices_do_not_collapse() {
        let h = road("road", &[(0.0, 0.0), (deg(3.0), 0.0), (deg(6.0), 0.0), (deg(500.0), 0.0)]);
        let g = RoadGraph::build(vec![h], R, DEFAULT_SNAP_M);
        assert_eq!(g.node_count(), 4);
    }

    #[test]
    fn t_junction_without_shared_vertex_is_split() {
        let h = road("road", &[(0.0, 0.0), (deg(1000.0), 0.0)]);
        // Ends 4 m north of the middle of `h`, no vertex there.
        let v = road("road", &[(deg(500.0), deg(800.0)), (deg(500.0), deg(4.0))]);
        let g = RoadGraph::build(vec![h, v], R, DEFAULT_SNAP_M);
        let from = g.locate(geo::lonlat_to_unit(0.0, 0.0)).unwrap();
        let to = g.locate(geo::lonlat_to_unit(deg(500.0), deg(800.0))).unwrap();
        let r = g.route(&from, &to).expect("connected through the T");
        assert!((r.length_m - 1300.0).abs() < 15.0, "len {}", r.length_m);
    }

    #[test]
    fn route_is_dense_and_follows_the_network() {
        // An L: east 1 km then north 1 km; the goal is beside the far end.
        let l = road("road", &[(0.0, 0.0), (deg(1000.0), 0.0), (deg(1000.0), deg(1000.0))]);
        let g = RoadGraph::build(vec![l], R, DEFAULT_SNAP_M);
        let from = g.locate(geo::lonlat_to_unit(deg(10.0), deg(20.0))).unwrap();
        assert!((from.offset_m - 20.0).abs() < 0.5, "offset {}", from.offset_m);
        let to = g.locate(geo::lonlat_to_unit(deg(1030.0), deg(990.0))).unwrap();
        let r = g.route(&from, &to).unwrap();
        assert!((r.length_m - 1980.0).abs() < 2.0, "len {}", r.length_m);
        let max_gap = r.points.windows(2).map(|w| w[1].s_m - w[0].s_m).fold(0.0, f64::max);
        assert!(max_gap <= ROUTE_STEP_M + 1e-6, "gap {max_gap}");
    }

    #[test]
    fn router_prefers_the_cheaper_road_type() {
        // Two ways from A to B: a trail straight across (1.4 km) and a road
        // around (2·424 + 800 ≈ 1.65 km). Trail cost 1.4 × 1.6 = 2.24 > 1.65.
        let a = (0.0, 0.0);
        let b = (deg(1400.0), 0.0);
        let trail = road("trail", &[a, b]);
        let r = road("road", &[a, (deg(300.0), deg(300.0)), (deg(1100.0), deg(300.0)), b]);
        let g = RoadGraph::build(vec![trail, r], R, DEFAULT_SNAP_M);
        let from = g.locate(geo::lonlat_to_unit(a.0, a.1 - deg(1.0))).unwrap();
        let to = g.locate(geo::lonlat_to_unit(b.0, b.1 - deg(1.0))).unwrap();
        let route = g.route(&from, &to).unwrap();
        assert!(route.length_m > 1600.0, "took the trail: {}", route.length_m);
    }

    #[test]
    fn disconnected_networks_have_no_route() {
        let a = road("road", &[(0.0, 0.0), (deg(100.0), 0.0)]);
        let b = road("road", &[(deg(1000.0), 0.0), (deg(1100.0), 0.0)]);
        let g = RoadGraph::build(vec![a, b], R, DEFAULT_SNAP_M);
        let from = g.locate(geo::lonlat_to_unit(0.0, 0.0)).unwrap();
        let to = g.locate(geo::lonlat_to_unit(deg(1100.0), 0.0)).unwrap();
        assert!(g.route(&from, &to).is_none());
        assert!(g.costs_to(&from, &[to])[0].is_infinite());
    }
}
