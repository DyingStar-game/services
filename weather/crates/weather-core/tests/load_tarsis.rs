//! Load the real Tarsis system (the celestial service's data file) with the climate extras and
//! derive every body. Prints the table the designers want to see; asserts what must hold.

use std::path::Path;
use weather_core::bodies::load;
use weather_core::climate::{derive, BodyKind};

fn read(path: &str) -> Option<String> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(p).ok()
}

#[test]
fn the_tarsis_system_derives_for_every_body() {
    let Some(system) = read("../../../resourcesDynamic/data/system/tarsis.json") else {
        eprintln!("tarsis.json not found beside this checkout: skipping");
        return;
    };
    let extras = read("../../data/climate/tarsis.json");
    let bodies = load(&system, extras.as_deref()).expect("load");
    assert_eq!(bodies.len(), 19, "8 planets + 11 moons");
    println!("{:<10} {:>5} {:>6} {:>7} {:>6} {:>5} {:>6} {:>5} {:>6} {:>6}", "body", "g", "P_bar", "H_km", "rho0", "Teq", "Ts", "phi_H", "cells", "v10thr");
    for b in &bodies {
        let c = derive(b);
        println!(
            "{:<10} {:>5.2} {:>6.2} {:>7.1} {:>6.3} {:>5.0} {:>6.0} {:>5.0} {:>6} {:>6.1}",
            c.name, c.surface_gravity, c.pressure_pa / 1e5, c.scale_height_m / 1e3, c.rho0_kg_m3, c.t_eq_k, c.t_surface_k, c.hadley_edge_deg, c.cells, c.v10_threshold_m_s
        );
        assert!(c.surface_gravity > 0.0 && c.surface_gravity < 50.0, "{}", c.name);
        assert!(c.t_surface_k > 0.0, "{}", c.name);
        if c.has_atmosphere {
            assert!(c.scale_height_m > 1_000.0 && c.scale_height_m < 200_000.0, "{} H={}", c.name, c.scale_height_m);
            assert!(c.rho0_kg_m3 > 0.0, "{}", c.name);
        }
    }
    let sandbox = bodies.iter().find(|b| b.name.eq_ignore_ascii_case("sandbox")).expect("SandBox");
    let c = derive(sandbox);
    assert_eq!(c.seed, 20261006, "seed from data/climate/tarsis.json");
    assert_eq!(c.kind, BodyKind::Planet);
    assert!((c.scale_height_m - 10_444.0).abs() < 12.0, "{}", c.scale_height_m);
    assert!((c.extras.dust_grain_density_kg_m3 - 4000.0).abs() < 1.0, "corundum from the extras file");
    let korax = bodies.iter().find(|b| b.name == "P3_M1").expect("Korax");
    assert!(!derive(korax).has_atmosphere);
    assert_eq!(korax.kind, BodyKind::Moon);
}
