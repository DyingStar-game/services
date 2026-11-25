//! Icosahedral geodesic grid and its dual hexagonal / pentagonal (Voronoi) cells.
//!
//! * Cell centres are the vertices of a recursively subdivided icosahedron
//!   (level L gives 10 * 4^L + 2 cells: 12 pentagons, the rest hexagons).
//! * Cell corners are the spherical circumcentres of the triangles, so the cells
//!   are the spherical Voronoi cells of the centres: each cell face (a "dual edge")
//!   is perpendicular to the segment joining the two centres and crosses it at
//!   its midpoint.
//! * Everything (areas, lengths, normals, neighbour lists) is computed once.

use crate::vec3::*;
use std::collections::HashMap;

pub struct Grid {
    pub level: u32,
    pub radius: f64,
    /// Cell centres, unit vectors.
    pub x: Vec<V3>,
    pub lon: Vec<f64>,
    pub lat: Vec<f64>,
    /// Cell areas in m^2.
    pub area: Vec<f64>,

    // ---- Edges (one per pair of neighbouring cells, i < j) ----
    pub e_i: Vec<usize>,
    pub e_j: Vec<usize>,
    /// Unit normal at the edge midpoint, tangent to the sphere, pointing from cell i to cell j.
    pub e_n: Vec<V3>,
    /// Unit tangent along the cell face: t = r x n (counter-clockwise around cell i).
    pub e_t: Vec<V3>,
    /// Length of the cell face (dual edge) in m.
    pub e_l: Vec<f64>,
    /// Distance between the two cell centres in m.
    pub e_d: Vec<f64>,
    /// Midpoint of the centre-to-centre segment (unit vector): where face values live.
    pub e_mid: Vec<V3>,
    /// Face end points (unit vectors), ordered so that t points from `e_a` to `e_b`.
    pub e_a: Vec<V3>,
    pub e_b: Vec<V3>,
    /// Position of this edge in the CSR lists of cell i and of cell j.
    pub e_slot_i: Vec<usize>,
    pub e_slot_j: Vec<usize>,

    // ---- Per-cell CSR lists, counter-clockwise ----
    pub c_start: Vec<usize>,
    pub c_edge: Vec<usize>,
    /// +1 if the cell is `e_i` of that edge (normal points outward), -1 otherwise.
    pub c_sign: Vec<f64>,
    pub c_nbr: Vec<usize>,
    /// Tangent-plane displacement (m) from the cell centre to the face midpoint.
    pub c_r: Vec<V3>,

    /// Triangles of the primal (Delaunay) mesh, counter-clockwise seen from outside.
    pub tris: Vec<[usize; 3]>,
    /// Triangles around each cell, CSR with `c_start` (triangle k = (cell, nbr k, nbr k+1)).
    pub c_tri: Vec<usize>,
}

fn icosahedron() -> (Vec<V3>, Vec<[usize; 3]>) {
    let p = (1.0 + 5f64.sqrt()) / 2.0;
    let mut v: Vec<V3> = Vec::new();
    for &a in &[-1.0, 1.0] {
        for &b in &[-p, p] {
            v.push([0.0, a, b]);
            v.push([a, b, 0.0]);
            v.push([b, 0.0, a]);
        }
    }
    let v: Vec<V3> = v.into_iter().map(normalize).collect();
    // Faces: all triples whose three pairwise distances equal the edge length.
    let mut emin = f64::MAX;
    for i in 0..12 {
        for j in 0..12 {
            if i != j {
                emin = emin.min(norm(sub(v[i], v[j])));
            }
        }
    }
    let close = |a: usize, b: usize| (norm(sub(v[a], v[b])) - emin).abs() < 1e-9;
    let mut f = Vec::new();
    for i in 0..12 {
        for j in i + 1..12 {
            for k in j + 1..12 {
                if close(i, j) && close(j, k) && close(i, k) {
                    let n = cross(sub(v[j], v[i]), sub(v[k], v[i]));
                    if dot(n, add(add(v[i], v[j]), v[k])) > 0.0 {
                        f.push([i, j, k]);
                    } else {
                        f.push([i, k, j]);
                    }
                }
            }
        }
    }
    assert_eq!(f.len(), 20);
    (v, f)
}

/// Area of the spherical triangle (unit sphere) with unit-vector corners a, b, c.
pub fn sph_tri_area(a: V3, b: V3, c: V3) -> f64 {
    let num = dot(a, cross(b, c)).abs();
    let den = 1.0 + dot(a, b) + dot(b, c) + dot(c, a);
    2.0 * num.atan2(den)
}

impl Grid {
    pub fn new(level: u32, radius: f64) -> Grid {
        let (mut v, mut f) = icosahedron();
        for _ in 0..level {
            let mut cache: HashMap<(usize, usize), usize> = HashMap::new();
            let mut nf = Vec::with_capacity(f.len() * 4);
            let mut mid = |a: usize, b: usize, v: &mut Vec<V3>| -> usize {
                let key = (a.min(b), a.max(b));
                *cache.entry(key).or_insert_with(|| {
                    v.push(normalize(add(v[a], v[b])));
                    v.len() - 1
                })
            };
            for t in &f {
                let ab = mid(t[0], t[1], &mut v);
                let bc = mid(t[1], t[2], &mut v);
                let ca = mid(t[2], t[0], &mut v);
                nf.push([t[0], ab, ca]);
                nf.push([t[1], bc, ab]);
                nf.push([t[2], ca, bc]);
                nf.push([ab, bc, ca]);
            }
            f = nf;
        }
        let n = v.len();
        // Directed edge (a -> b) -> triangle on its left.
        let mut left: HashMap<(usize, usize), usize> = HashMap::with_capacity(f.len() * 3);
        for (ti, t) in f.iter().enumerate() {
            left.insert((t[0], t[1]), ti);
            left.insert((t[1], t[2]), ti);
            left.insert((t[2], t[0]), ti);
        }
        let third = |t: &[usize; 3], a: usize, b: usize| -> usize {
            *t.iter().find(|&&q| q != a && q != b).unwrap()
        };
        // One starting neighbour per vertex.
        let mut first = vec![usize::MAX; n];
        for t in &f {
            first[t[0]] = t[1];
            first[t[1]] = t[2];
            first[t[2]] = t[0];
        }
        // Spherical circumcentres of the triangles.
        let cc: Vec<V3> = f
            .iter()
            .map(|t| normalize(cross(sub(v[t[1]], v[t[0]]), sub(v[t[2]], v[t[0]]))))
            .collect();

        let mut c_start = vec![0usize; n + 1];
        let mut c_nbr = Vec::with_capacity(n * 6);
        let mut c_tri = Vec::with_capacity(n * 6);
        for i in 0..n {
            let n0 = first[i];
            let mut cur = n0;
            loop {
                let ti = left[&(i, cur)];
                c_nbr.push(cur);
                c_tri.push(ti);
                cur = third(&f[ti], i, cur);
                if cur == n0 {
                    break;
                }
            }
            c_start[i + 1] = c_nbr.len();
        }

        let lon: Vec<f64> = v.iter().map(|p| to_lonlat(*p).0).collect();
        let lat: Vec<f64> = v.iter().map(|p| to_lonlat(*p).1).collect();
        let r2 = radius * radius;
        let mut area = vec![0.0; n];
        for i in 0..n {
            let (s, e) = (c_start[i], c_start[i + 1]);
            let k = e - s;
            let mut a = 0.0;
            for q in 0..k {
                let c0 = cc[c_tri[s + (q + k - 1) % k]];
                let c1 = cc[c_tri[s + q]];
                a += sph_tri_area(v[i], c0, c1);
            }
            area[i] = a * r2;
        }

        // Edges.
        let mut eid: HashMap<(usize, usize), usize> = HashMap::new();
        let mut g = Grid {
            level,
            radius,
            x: v.clone(),
            lon,
            lat,
            area,
            e_i: vec![],
            e_j: vec![],
            e_n: vec![],
            e_t: vec![],
            e_l: vec![],
            e_d: vec![],
            e_mid: vec![],
            e_a: vec![],
            e_b: vec![],
            e_slot_i: vec![],
            e_slot_j: vec![],
            c_start: c_start.clone(),
            c_edge: vec![0; c_nbr.len()],
            c_sign: vec![0.0; c_nbr.len()],
            c_nbr: c_nbr.clone(),
            c_r: vec![[0.0; 3]; c_nbr.len()],
            tris: f.clone(),
            c_tri: c_tri.clone(),
        };
        for i in 0..n {
            let (s, e) = (c_start[i], c_start[i + 1]);
            let k = e - s;
            for q in 0..k {
                let j = c_nbr[s + q];
                if i < j {
                    let m = normalize(add(v[i], v[j]));
                    let nn = normalize(tangent(sub(v[j], v[i]), m));
                    let tt = cross(m, nn);
                    // Face from the circumcentre of triangle (i, nbr q-1, nbr q)
                    // to that of triangle (i, nbr q, nbr q+1): this runs along +t.
                    let a = cc[c_tri[s + (q + k - 1) % k]];
                    let b = cc[c_tri[s + q]];
                    debug_assert!(dot(sub(b, a), tt) > 0.0);
                    eid.insert((i, j), g.e_i.len());
                    g.e_i.push(i);
                    g.e_j.push(j);
                    g.e_n.push(nn);
                    g.e_t.push(tt);
                    g.e_l.push(angle(a, b) * radius);
                    g.e_d.push(angle(v[i], v[j]) * radius);
                    g.e_mid.push(m);
                    g.e_a.push(a);
                    g.e_b.push(b);
                    g.e_slot_i.push(0);
                    g.e_slot_j.push(0);
                }
            }
        }
        for i in 0..n {
            let (s, e) = (c_start[i], c_start[i + 1]);
            for slot in s..e {
                let j = c_nbr[slot];
                let (ed, sign) = if i < j {
                    (eid[&(i, j)], 1.0)
                } else {
                    (eid[&(j, i)], -1.0)
                };
                g.c_edge[slot] = ed;
                g.c_sign[slot] = sign;
                if sign > 0.0 {
                    g.e_slot_i[ed] = slot;
                } else {
                    g.e_slot_j[ed] = slot;
                }
                // Gnomonic projection of the face midpoint on the tangent plane at x_i.
                let m = g.e_mid[ed];
                g.c_r[slot] = scale(sub(scale(m, 1.0 / dot(m, v[i])), v[i]), radius);
            }
        }
        g
    }

    pub fn n_cells(&self) -> usize {
        self.x.len()
    }
    pub fn n_edges(&self) -> usize {
        self.e_i.len()
    }
    pub fn mean_spacing(&self) -> f64 {
        self.e_d.iter().sum::<f64>() / self.e_d.len() as f64
    }
    pub fn min_spacing(&self) -> f64 {
        self.e_d.iter().cloned().fold(f64::MAX, f64::min)
    }

    /// Area-weighted integral of a cell field (m^2 * unit).
    pub fn integral(&self, f: &[f64]) -> f64 {
        f.iter().zip(&self.area).map(|(a, b)| a * b).sum()
    }

    /// Nearest cell centre to the unit vector `p`, by a greedy walk from `start`
    /// (on a Delaunay mesh the greedy walk always ends at the nearest vertex).
    pub fn nearest(&self, p: V3, start: usize) -> usize {
        let mut best = start;
        let mut bd = dot(self.x[best], p);
        loop {
            let mut moved = false;
            for s in self.c_start[best]..self.c_start[best + 1] {
                let j = self.c_nbr[s];
                let d = dot(self.x[j], p);
                if d > bd {
                    bd = d;
                    best = j;
                    moved = true;
                }
            }
            if !moved {
                return best;
            }
        }
    }

    /// Linear (barycentric) interpolation stencil of the point `p` on the primal triangles.
    /// Returns the three cells and their weights, plus the nearest cell (as a walk hint).
    pub fn locate(&self, p: V3, start: usize) -> ([usize; 3], [f64; 3], usize) {
        let nc = self.nearest(p, start);
        let mut cands: Vec<usize> = (self.c_start[nc]..self.c_start[nc + 1])
            .map(|s| self.c_tri[s])
            .collect();
        for s in self.c_start[nc]..self.c_start[nc + 1] {
            let j = self.c_nbr[s];
            for q in self.c_start[j]..self.c_start[j + 1] {
                cands.push(self.c_tri[q]);
            }
        }
        for &ti in &cands {
            let t = self.tris[ti];
            let (a, b, c) = (self.x[t[0]], self.x[t[1]], self.x[t[2]]);
            // Solve p = wa a + wb b + wc c (Cramer), then normalise the weights.
            let det = dot(a, cross(b, c));
            let wa = dot(p, cross(b, c)) / det;
            let wb = dot(a, cross(p, c)) / det;
            let wc = dot(a, cross(b, p)) / det;
            let eps = -1e-10;
            if wa >= eps && wb >= eps && wc >= eps {
                let s = wa + wb + wc;
                return (t, [wa / s, wb / s, wc / s], nc);
            }
        }
        ([nc, nc, nc], [1.0, 0.0, 0.0], nc)
    }
}
