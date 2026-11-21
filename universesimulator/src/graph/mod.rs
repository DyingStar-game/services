//! Weighted warehouse route graph and shortest paths (ADR 0006).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

/// A route as stored in `travel_routes`.
#[derive(Debug, Clone)]
pub struct RouteEdge {
    pub from: Uuid,
    pub to: Uuid,
    pub cost: f64,
    pub distance_m: f64,
    pub bidirectional: bool,
}

#[derive(Debug, Clone, Copy)]
struct Edge {
    to: Uuid,
    cost: f64,
    distance_m: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hop {
    pub from: Uuid,
    pub to: Uuid,
    pub cost: f64,
    pub distance_m: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Path {
    pub hops: Vec<Hop>,
    pub total_cost: f64,
    pub total_distance_m: f64,
}

impl Path {
    /// Warehouses along the path, origin first.
    pub fn nodes(&self) -> Vec<Uuid> {
        let mut out: Vec<Uuid> = self.hops.iter().map(|h| h.from).collect();
        out.extend(self.hops.last().map(|h| h.to));
        out
    }

    fn from_hops(hops: Vec<Hop>) -> Self {
        Self {
            total_cost: hops.iter().map(|h| h.cost).sum(),
            total_distance_m: hops.iter().map(|h| h.distance_m).sum(),
            hops,
        }
    }
}

#[derive(Debug, Default)]
pub struct Graph {
    forward: HashMap<Uuid, Vec<Edge>>,
    backward: HashMap<Uuid, Vec<Edge>>,
}

#[derive(PartialEq)]
struct State {
    cost: f64,
    node: Uuid,
}

impl Eq for State {}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on cost; ties broken on the node id for determinism.
        other.cost.total_cmp(&self.cost).then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Result of a single-source Dijkstra: distances and the edge used to reach each node.
struct Tree {
    dist: HashMap<Uuid, f64>,
    prev: HashMap<Uuid, (Uuid, Edge)>,
    order: Vec<Uuid>,
}

impl Graph {
    /// Builds the graph from active warehouses; routes touching unknown nodes are ignored.
    pub fn new(nodes: &HashSet<Uuid>, routes: &[RouteEdge]) -> Self {
        let mut g = Self::default();
        for n in nodes {
            g.forward.entry(*n).or_default();
            g.backward.entry(*n).or_default();
        }
        for r in routes.iter().filter(|r| nodes.contains(&r.from) && nodes.contains(&r.to)) {
            g.add(r.from, r.to, r.cost, r.distance_m);
            if r.bidirectional {
                g.add(r.to, r.from, r.cost, r.distance_m);
            }
        }
        g
    }

    fn add(&mut self, from: Uuid, to: Uuid, cost: f64, distance_m: f64) {
        self.forward.entry(from).or_default().push(Edge { to, cost, distance_m });
        self.backward.entry(to).or_default().push(Edge { to: from, cost, distance_m });
    }

    pub fn contains(&self, n: Uuid) -> bool {
        self.forward.contains_key(&n)
    }

    fn dijkstra(adj: &HashMap<Uuid, Vec<Edge>>, source: Uuid) -> Tree {
        let mut tree = Tree { dist: HashMap::new(), prev: HashMap::new(), order: Vec::new() };
        let mut heap = BinaryHeap::new();
        tree.dist.insert(source, 0.0);
        heap.push(State { cost: 0.0, node: source });
        let mut done = HashSet::new();
        while let Some(State { cost, node }) = heap.pop() {
            if !done.insert(node) {
                continue;
            }
            tree.order.push(node);
            for e in adj.get(&node).into_iter().flatten() {
                let next = cost + e.cost;
                if tree.dist.get(&e.to).is_none_or(|d| next < *d) {
                    tree.dist.insert(e.to, next);
                    tree.prev.insert(e.to, (node, *e));
                    heap.push(State { cost: next, node: e.to });
                }
            }
        }
        tree
    }

    /// Cheapest path from `from` to `to`; `None` when unreachable or identical.
    pub fn shortest_path(&self, from: Uuid, to: Uuid) -> Option<Path> {
        if from == to || !self.contains(from) || !self.contains(to) {
            return None;
        }
        let tree = Self::dijkstra(&self.forward, from);
        let mut hops = Vec::new();
        let mut cur = to;
        while cur != from {
            let (p, e) = tree.prev.get(&cur)?;
            hops.push(Hop { from: *p, to: cur, cost: e.cost, distance_m: e.distance_m });
            cur = *p;
        }
        hops.reverse();
        Some(Path::from_hops(hops))
    }

    /// Closest node (by path cost towards `dest`) accepted by `accept`, with its path to `dest`.
    /// Runs one Dijkstra on the reversed graph from `dest`.
    pub fn nearest_source(&self, dest: Uuid, mut accept: impl FnMut(Uuid) -> bool) -> Option<Path> {
        if !self.contains(dest) {
            return None;
        }
        let tree = Self::dijkstra(&self.backward, dest);
        let source = tree.order.iter().copied().find(|n| *n != dest && accept(*n))?;
        // In the reversed tree, `prev[n]` is the next node towards `dest`.
        let mut hops = Vec::new();
        let mut cur = source;
        while cur != dest {
            let (next, e) = tree.prev.get(&cur)?;
            hops.push(Hop { from: cur, to: *next, cost: e.cost, distance_m: e.distance_m });
            cur = *next;
        }
        Some(Path::from_hops(hops))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<Uuid> {
        (0..n).map(|i| Uuid::from_u128(i as u128 + 1)).collect()
    }

    fn edge(a: Uuid, b: Uuid, cost: f64) -> RouteEdge {
        RouteEdge { from: a, to: b, cost, distance_m: cost * 1000.0, bidirectional: true }
    }

    #[test]
    fn chain_village_town_city() {
        let n = ids(3);
        let g = Graph::new(&n.iter().copied().collect(), &[edge(n[0], n[1], 2.0), edge(n[1], n[2], 3.0)]);
        let p = g.shortest_path(n[0], n[2]).unwrap();
        assert_eq!(p.nodes(), vec![n[0], n[1], n[2]]);
        assert_eq!(p.total_cost, 5.0);
        assert_eq!(p.total_distance_m, 5000.0);
        // Bidirectional: the way back exists too.
        assert_eq!(g.shortest_path(n[2], n[0]).unwrap().hops.len(), 2);
    }

    #[test]
    fn shortcut_wins() {
        let n = ids(3);
        let routes = [edge(n[0], n[1], 2.0), edge(n[1], n[2], 3.0), edge(n[0], n[2], 4.0)];
        let g = Graph::new(&n.iter().copied().collect(), &routes);
        assert_eq!(g.shortest_path(n[0], n[2]).unwrap().nodes(), vec![n[0], n[2]]);
    }

    #[test]
    fn one_way_route() {
        let n = ids(2);
        let mut r = edge(n[0], n[1], 1.0);
        r.bidirectional = false;
        let g = Graph::new(&n.iter().copied().collect(), &[r]);
        assert!(g.shortest_path(n[0], n[1]).is_some());
        assert!(g.shortest_path(n[1], n[0]).is_none());
    }

    #[test]
    fn disconnected_and_unknown() {
        let n = ids(3);
        let g = Graph::new(&n[..2].iter().copied().collect(), &[edge(n[0], n[2], 1.0)]);
        assert!(g.shortest_path(n[0], n[1]).is_none());
        assert!(g.shortest_path(n[0], n[2]).is_none());
        assert!(g.shortest_path(n[0], n[0]).is_none());
    }

    #[test]
    fn cycles_terminate() {
        let n = ids(4);
        let routes = [edge(n[0], n[1], 1.0), edge(n[1], n[2], 1.0), edge(n[2], n[0], 1.0), edge(n[2], n[3], 1.0)];
        let g = Graph::new(&n.iter().copied().collect(), &routes);
        assert_eq!(g.shortest_path(n[0], n[3]).unwrap().total_cost, 2.0);
    }

    #[test]
    fn nearest_source_prefers_cheapest_supplier() {
        // dest(0) - 1 - 2 ; dest - 3 (expensive). Suppliers: 2 and 3.
        let n = ids(4);
        let routes = [edge(n[0], n[1], 1.0), edge(n[1], n[2], 1.0), edge(n[0], n[3], 5.0)];
        let g = Graph::new(&n.iter().copied().collect(), &routes);
        let p = g.nearest_source(n[0], |w| w == n[2] || w == n[3]).unwrap();
        assert_eq!(p.nodes(), vec![n[2], n[1], n[0]]);
        assert!(g.nearest_source(n[0], |_| false).is_none());
    }
}
