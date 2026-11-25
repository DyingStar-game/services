//! Tiny 3-vector helpers on `[f64; 3]` (no external linear-algebra crate needed).

pub type V3 = [f64; 3];

#[inline]
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
pub fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
#[inline]
pub fn normalize(a: V3) -> V3 {
    scale(a, 1.0 / norm(a))
}
/// Removes the component of `w` along the unit vector `r` (projection on the tangent plane).
#[inline]
pub fn tangent(w: V3, r: V3) -> V3 {
    sub(w, scale(r, dot(w, r)))
}
/// Angle between two unit vectors, accurate for small angles.
#[inline]
pub fn angle(a: V3, b: V3) -> f64 {
    norm(cross(a, b)).atan2(dot(a, b))
}
/// Unit position vector from longitude / latitude in radians.
#[inline]
pub fn from_lonlat(lon: f64, lat: f64) -> V3 {
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}
/// (longitude, latitude) in radians of a unit vector; longitude in -pi..pi.
#[inline]
pub fn to_lonlat(p: V3) -> (f64, f64) {
    (p[1].atan2(p[0]), p[2].clamp(-1.0, 1.0).asin())
}
/// Local unit vectors pointing east and north at `p` (undefined exactly at the poles,
/// where an arbitrary but finite frame is returned).
#[inline]
pub fn east_north(p: V3) -> (V3, V3) {
    let (lon, lat) = to_lonlat(p);
    let e = [-lon.sin(), lon.cos(), 0.0];
    let n = [-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos()];
    (e, n)
}
