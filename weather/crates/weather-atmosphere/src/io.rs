//! Input (relief raster) and output (equirectangular 16-bit PNG tables + JSON sidecar).

use crate::grid::Grid;
use crate::vec3::*;
use std::f64::consts::PI;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

/// Reads a raw little-endian float32 raster (row-major, `w` x `h`).
pub fn read_f32_raster(path: &Path, w: usize, h: usize) -> std::io::Result<Vec<f32>> {
    let mut buf = Vec::new();
    File::open(path)?.read_to_end(&mut buf)?;
    if buf.len() != w * h * 4 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected {} bytes, got {}", w * h * 4, buf.len()),
        ));
    }
    Ok(buf.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

/// Unit vector at the centre of pixel (col, row) of an equirectangular raster
/// (longitude -180..180 left to right, latitude 90..-90 top to bottom).
pub fn pixel_dir(col: usize, row: usize, w: usize, h: usize) -> V3 {
    let lon = -PI + (col as f64 + 0.5) * 2.0 * PI / w as f64;
    let lat = PI / 2.0 - (row as f64 + 0.5) * PI / h as f64;
    from_lonlat(lon, lat)
}

/// Bilinear sample of an equirectangular raster at (lon, lat) in radians.
pub fn bilinear(r: &[f32], w: usize, h: usize, lon: f64, lat: f64) -> f64 {
    let x = (lon + PI) / (2.0 * PI) * w as f64 - 0.5;
    let y = (PI / 2.0 - lat) / PI * h as f64 - 0.5;
    let x0 = x.floor();
    let y0 = y.floor().clamp(0.0, h as f64 - 1.0);
    let fx = x - x0;
    let fy = (y - y0).clamp(0.0, 1.0);
    let xi = |k: f64| (((k as i64) % w as i64 + w as i64) % w as i64) as usize;
    let yi = |k: f64| (k as i64).clamp(0, h as i64 - 1) as usize;
    let at = |cx: f64, cy: f64| r[yi(cy) * w + xi(cx)] as f64;
    (1.0 - fy) * ((1.0 - fx) * at(x0, y0) + fx * at(x0 + 1.0, y0))
        + fy * ((1.0 - fx) * at(x0, y0 + 1.0) + fx * at(x0 + 1.0, y0 + 1.0))
}

/// Cell averages of a raster: every pixel goes to its nearest cell, weighted by
/// its true area (cos latitude). Cells that receive no pixel (very fine grids)
/// fall back to a bilinear sample at their centre.
pub fn raster_to_cells(grid: &Grid, r: &[f32], w: usize, h: usize) -> Vec<f64> {
    let n = grid.n_cells();
    let mut sum = vec![0.0; n];
    let mut wsum = vec![0.0; n];
    let mut hint = 0;
    for row in 0..h {
        for col in 0..w {
            let p = pixel_dir(col, row, w, h);
            let c = grid.nearest(p, hint);
            hint = c;
            let wt = p[2].clamp(-1.0, 1.0).asin().cos();
            sum[c] += wt * r[row * w + col] as f64;
            wsum[c] += wt;
        }
    }
    (0..n)
        .map(|i| {
            if wsum[i] > 0.0 {
                sum[i] / wsum[i]
            } else {
                bilinear(r, w, h, grid.lon[i], grid.lat[i])
            }
        })
        .collect()
}

/// Precomputed linear interpolation from cells to an equirectangular image.
pub struct Sampler {
    pub w: usize,
    pub h: usize,
    idx: Vec<[usize; 3]>,
    wt: Vec<[f64; 3]>,
    east: Vec<V3>,
    north: Vec<V3>,
}

impl Sampler {
    pub fn new(grid: &Grid, w: usize, h: usize) -> Sampler {
        let mut idx = Vec::with_capacity(w * h);
        let mut wt = Vec::with_capacity(w * h);
        let mut east = Vec::with_capacity(w * h);
        let mut north = Vec::with_capacity(w * h);
        let mut hint = 0;
        for row in 0..h {
            for col in 0..w {
                let p = pixel_dir(col, row, w, h);
                let (t, ws, nc) = grid.locate(p, hint);
                hint = nc;
                idx.push(t);
                wt.push(ws);
                let (e, n) = east_north(p);
                east.push(e);
                north.push(n);
            }
        }
        Sampler { w, h, idx, wt, east, north }
    }

    pub fn scalar(&self, f: &[f64]) -> Vec<f32> {
        self.idx
            .iter()
            .zip(&self.wt)
            .map(|(t, w)| (w[0] * f[t[0]] + w[1] * f[t[1]] + w[2] * f[t[2]]) as f32)
            .collect()
    }

    /// Interpolates the Cartesian vector, then projects on local east / north.
    pub fn east_north(&self, v: &[V3]) -> (Vec<f32>, Vec<f32>) {
        let mut ue = Vec::with_capacity(self.idx.len());
        let mut vn = Vec::with_capacity(self.idx.len());
        for k in 0..self.idx.len() {
            let (t, w) = (self.idx[k], self.wt[k]);
            let mut a = [0.0; 3];
            for q in 0..3 {
                a = add(a, scale(v[t[q]], w[q]));
            }
            ue.push(dot(a, self.east[k]) as f32);
            vn.push(dot(a, self.north[k]) as f32);
        }
        (ue, vn)
    }
}

/// Description of one output table for the JSON sidecar.
pub struct TableMeta<'a> {
    pub field: &'a str,
    pub units: &'a str,
    pub description: &'a str,
    pub time_s: f64,
    pub time_hours: f64,
    pub planet_day_s: f64,
    pub grid_level: u32,
    pub grid_cells: usize,
    pub radius_m: f64,
    pub fixed_range: Option<(f64, f64)>,
}

/// One table encoded in memory: the 16-bit PNG, its JSON sidecar, and the range of the values.
pub struct EncodedTable {
    pub png: Vec<u8>,
    pub json: String,
    pub min: f64,
    pub max: f64,
}

/// Quantises `data` to 16 bits (linear between min and max), writes a grayscale
/// PNG and a JSON sidecar with the same stem. Returns (min, max).
pub fn write_table(path_png: &Path, w: usize, h: usize, data: &[f32], meta: &TableMeta) -> std::io::Result<(f64, f64)> {
    let t = encode_table(w, h, data, meta)?;
    File::create(path_png)?.write_all(&t.png)?;
    File::create(path_png.with_extension("json"))?.write_all(t.json.as_bytes())?;
    Ok((t.min, t.max))
}

/// Same as `write_table`, in memory.
pub fn encode_table(w: usize, h: usize, data: &[f32], meta: &TableMeta) -> std::io::Result<EncodedTable> {
    let (mut lo, mut hi) = match meta.fixed_range {
        Some(r) => r,
        None => {
            let lo = data.iter().cloned().fold(f32::MAX, f32::min) as f64;
            let hi = data.iter().cloned().fold(f32::MIN, f32::max) as f64;
            (lo, hi)
        }
    };
    if !(hi > lo) {
        hi = lo + 1.0;
        lo -= 0.0;
    }
    let mut bytes = Vec::with_capacity(w * h * 2);
    for &x in data {
        let q = (((x as f64 - lo) / (hi - lo)).clamp(0.0, 1.0) * 65535.0).round() as u16;
        bytes.extend_from_slice(&q.to_be_bytes());
    }
    let mut png_bytes: Vec<u8> = Vec::with_capacity(w * h);
    {
        let mut enc = png::Encoder::new(BufWriter::new(&mut png_bytes), w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Sixteen);
        let mut wr = enc.write_header().map_err(std::io::Error::other)?;
        wr.write_image_data(&bytes).map_err(std::io::Error::other)?;
    }
    let json = format!(
        concat!(
            "{{\n",
            "  \"field\": \"{}\",\n",
            "  \"units\": \"{}\",\n",
            "  \"description\": \"{}\",\n",
            "  \"min\": {:.9e},\n",
            "  \"max\": {:.9e},\n",
            "  \"encoding\": \"value = min + (max - min) * pixel / 65535, 16-bit grayscale PNG\",\n",
            "  \"time_s\": {:.3},\n",
            "  \"time_hours\": {:.4},\n",
            "  \"time_planet_days\": {:.5},\n",
            "  \"width\": {},\n",
            "  \"height\": {},\n",
            "  \"projection\": \"equirectangular; columns: longitude -180..180 west to east; rows: latitude 90..-90 north to south; values at pixel centres\",\n",
            "  \"grid\": {{ \"type\": \"icosahedral hexagonal (Voronoi), linear interpolation on the triangles\", \"level\": {}, \"cells\": {}, \"radius_m\": {:.1} }}\n",
            "}}\n"
        ),
        meta.field,
        meta.units,
        meta.description,
        lo,
        hi,
        meta.time_s,
        meta.time_hours,
        meta.time_s / meta.planet_day_s,
        w,
        h,
        meta.grid_level,
        meta.grid_cells,
        meta.radius_m
    );
    Ok(EncodedTable { png: png_bytes, json, min: lo, max: hi })
}
