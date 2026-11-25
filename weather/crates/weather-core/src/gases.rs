//! Molar mass and specific heat of the gases the system data names, so an atmosphere's mean molar
//! mass and heat capacity follow from its composition instead of being guessed.

/// Molar mass (kg/mol) and specific heat at constant pressure (J kg⁻¹ K⁻¹, near 300 K; the
/// greenhouse flag says whether the gas absorbs thermal infrared) of one species.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gas {
    pub molar_mass_kg_mol: f64,
    pub cp_j_kg_k: f64,
    pub greenhouse: bool,
}

/// Look a species up by its symbol as written in `tarsis.json` (`N2`, `CO2`, `H2O`, ...). Unknown
/// species fall back on an air-like gas so a typo in the data degrades the model instead of
/// stopping it; the caller may log the fallback.
pub fn gas(symbol: &str) -> Option<Gas> {
    let g = |m: f64, cp: f64, greenhouse: bool| Gas { molar_mass_kg_mol: m * 1e-3, cp_j_kg_k: cp, greenhouse };
    Some(match symbol {
        "N2" => g(28.014, 1040.0, false),
        "O2" => g(31.998, 918.0, false),
        "CO2" => g(44.009, 846.0, true),
        "Ar" => g(39.948, 520.0, false),
        "CH4" => g(16.043, 2220.0, true),
        "H2" => g(2.016, 14300.0, false),
        "He" => g(4.0026, 5193.0, false),
        "H2O" => g(18.015, 1996.0, true),
        "NH3" => g(17.031, 2190.0, true),
        "SO2" => g(64.066, 640.0, true),
        "HCN" => g(27.025, 1330.0, true),
        "NO2" => g(46.005, 800.0, true),
        "O3" => g(47.998, 820.0, true),
        "Ne" => g(20.180, 1030.0, false),
        "CO" => g(28.010, 1040.0, false),
        _ => return None,
    })
}

/// Air-like fallback for an unknown species.
pub const FALLBACK: Gas = Gas { molar_mass_kg_mol: 29.0e-3, cp_j_kg_k: 1000.0, greenhouse: false };

/// Mean molar mass (kg/mol), mass-weighted specific heat (J kg⁻¹ K⁻¹) and the molar fraction of
/// greenhouse gases of a mixture given as (symbol, fraction in percent). Fractions are normalised,
/// so a composition that sums to 99.3 % is read as a whole.
pub fn mixture(gases_pct: &[(String, f64)]) -> Mixture {
    let total: f64 = gases_pct.iter().map(|(_, p)| p.max(0.0)).sum();
    if total <= 0.0 {
        return Mixture { molar_mass_kg_mol: FALLBACK.molar_mass_kg_mol, cp_j_kg_k: FALLBACK.cp_j_kg_k, greenhouse_fraction: 0.0, unknown: vec![] };
    }
    let mut molar_mass = 0.0;
    let mut mass_cp = 0.0;
    let mut greenhouse = 0.0;
    let mut unknown = Vec::new();
    for (symbol, pct) in gases_pct {
        let x = pct.max(0.0) / total;
        let g = match gas(symbol) {
            Some(g) => g,
            None => {
                unknown.push(symbol.clone());
                FALLBACK
            }
        };
        molar_mass += x * g.molar_mass_kg_mol;
        mass_cp += x * g.molar_mass_kg_mol * g.cp_j_kg_k;
        if g.greenhouse {
            greenhouse += x;
        }
    }
    Mixture { molar_mass_kg_mol: molar_mass, cp_j_kg_k: mass_cp / molar_mass, greenhouse_fraction: greenhouse, unknown }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mixture {
    pub molar_mass_kg_mol: f64,
    pub cp_j_kg_k: f64,
    /// Molar fraction (0–1) of infrared-absorbing gases.
    pub greenhouse_fraction: f64,
    /// Species the table does not know, read as air.
    pub unknown: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earth_air_is_29_grams_per_mole() {
        let air = mixture(&[("N2".into(), 78.08), ("O2".into(), 20.95), ("Ar".into(), 0.93), ("CO2".into(), 0.04)]);
        assert!((air.molar_mass_kg_mol * 1e3 - 28.96).abs() < 0.05, "{}", air.molar_mass_kg_mol);
        assert!((air.cp_j_kg_k - 1004.0).abs() < 15.0, "{}", air.cp_j_kg_k);
    }

    #[test]
    fn sandbox_air_matches_the_games_atmosphere_profile() {
        // build_atmosphere_profiles.gd derives 10 444 m for the scale height from this composition at
        // 283 K and 7.775 m/s²: M must be 28.98 g/mol.
        let air = mixture(&[("Ar".into(), 1.1), ("CH4".into(), 0.2), ("CO2".into(), 0.5), ("N2".into(), 78.7), ("O2".into(), 19.5)]);
        assert!((air.molar_mass_kg_mol * 1e3 - 28.98).abs() < 0.03, "{}", air.molar_mass_kg_mol);
    }

    #[test]
    fn an_unknown_gas_is_reported_not_fatal() {
        let m = mixture(&[("N2".into(), 90.0), ("Xx".into(), 10.0)]);
        assert_eq!(m.unknown, vec!["Xx".to_string()]);
    }
}
