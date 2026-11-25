//! The near-ground fields published to the game (`weather_field`): the grid convention of the
//! contract (column i at longitude -180 + (i + 0.5) 360 / w, increasing eastward; row j at latitude
//! 90 - (j + 0.5) 180 / h, row 0 = north), the eastward / northward wind components, the sizes.

mod common;

use weather_atmosphere::live::LiveModel;
use weather_atmosphere::setup::Scenario;
use weather_atmosphere::vec3::{east_north, from_lonlat, scale};
use weather_atmosphere::with_threads;

const W: usize = 128;
const H: usize = 64;

fn model() -> LiveModel {
    let mut sc = Scenario::for_body(&common::settings_dir(), "Sandbox", &[], &common::files()).unwrap();
    sc.s.grid.level = 4;
    LiveModel::cold_start(sc).unwrap()
}

fn pixel_lonlat(i: usize, j: usize) -> (f64, f64) {
    (-180.0 + (i as f64 + 0.5) * 360.0 / W as f64, 90.0 - (j as f64 + 0.5) * 180.0 / H as f64)
}

/// A warm spot put in one known cell (the players' starting area, lon -39, lat 22) comes out at
/// the pixel of that longitude and latitude.
#[test]
fn a_known_cell_lands_at_its_longitude_and_latitude() {
    let _g = common::heavy();
    with_threads(2, || {
        let mut m = model();
        let (lon, lat) = (-39.0f64, 22.0f64);
        let cell = m.model.grid.nearest(from_lonlat(lon.to_radians(), lat.to_radians()), 0);
        let l = m.model.nl;
        let before = m.near_ground(W, H)[3].clone();
        // +15 % potential temperature in the lowest layer of that cell: a ~40 K warm spot.
        m.st.th[cell * l + l - 1] *= 1.15;
        let q = m.near_ground(W, H);
        assert_eq!(q.len(), 5);
        assert!(q.iter().all(|v| v.len() == W * H), "every quantity is {W} x {H}");
        let t_air = &q[3];
        let warming: Vec<f32> = t_air.iter().zip(&before).map(|(a, b)| a - b).collect();
        let best = (0..W * H).max_by(|&a, &b| warming[a].partial_cmp(&warming[b]).unwrap()).unwrap();
        let (plon, plat) = pixel_lonlat(best % W, best / W);
        println!("warm cell lon {:.1} lat {:.1} -> hottest pixel ({}, {}) lon {plon:.1} lat {plat:.1}", m.model.grid.lon[cell].to_degrees(), m.model.grid.lat[cell].to_degrees(), best % W, best / W);
        // Level 4 cells are ~4 degrees apart, pixels 2.8 degrees.
        assert!((plon - lon).abs() < 5.0 && (plat - lat).abs() < 5.0, "hottest pixel at lon {plon} lat {plat}");
        // The temperatures are near-ground air temperatures, the pressure a sea-level pressure.
        assert!(t_air.iter().all(|&t| t > 150.0 && t < 400.0));
        assert!(q[4].iter().all(|&p| p > 5.0e4 && p < 2.0e5), "p_sea in Pa");
        assert!(q[2].iter().all(|&tau| tau >= 0.0));
    });
}

/// A uniform eastward wind in the lowest layer gives a positive `wind_east`, reduced to 10 m by the
/// log law, and no `wind_north`.
#[test]
fn eastward_wind_is_wind_east() {
    let _g = common::heavy();
    with_threads(2, || {
        let mut m = model();
        let l = m.model.nl;
        for i in 0..m.model.grid.n_cells() {
            let (east, _) = east_north(m.model.grid.x[i]);
            m.st.v[i * l + l - 1] = scale(east, 10.0);
        }
        let q = m.near_ground(W, H);
        // Away from the poles (where the east vector turns quickly between cells).
        for j in 8..H - 8 {
            for i in 0..W {
                let (ue, vn) = (q[0][j * W + i], q[1][j * W + i]);
                assert!(ue > 2.0 && ue < 10.0, "wind_east {ue} at ({i}, {j})");
                assert!(vn.abs() < 0.05 * ue, "wind_north {vn} at ({i}, {j})");
            }
        }
    });
}
