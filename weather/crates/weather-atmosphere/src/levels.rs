//! Vertical levels from the body's air column: a rule instead of per-planet hand tables.
//!
//! The half levels are laid in the log-pressure height z = -H ln(eta) (eta = p / p_surface,
//! H = scale height): the highest interior half level sits at `top_factor` x tropopause height and
//! the j-th interior half level from the ground (j = 1 .. N-1) at (j / (N-1))^`stretch` of that
//! height, so the layers are thin near the ground and thicken upward, and the tropopause falls
//! inside the upper layers on any body. The terrain-following part of the hybrid coordinate stops
//! at the interior half level nearest `hybrid_factor` x tropopause height (below the tropopause,
//! so that the layers holding it are flat pressure surfaces).

/// Half levels (top = 0, ground = 1) from the level rule.
pub fn rule_eta_half(n: usize, scale_height: f64, tropopause: f64, top_factor: f64, stretch: f64) -> Vec<f64> {
    assert!(n >= 2, "at least 2 layers");
    let ztop = top_factor * tropopause;
    let mut e = vec![0.0; n + 1];
    e[n] = 1.0;
    for j in 1..n {
        let z = ztop * (j as f64 / (n - 1) as f64).powf(stretch);
        e[n - j] = (-z / scale_height).exp();
    }
    e
}

/// Top of the terrain-following part: the interior half level nearest `target` (in eta).
pub fn nearest_interior(eta_half: &[f64], target: f64) -> f64 {
    let n = eta_half.len() - 1;
    eta_half[1..n].iter().cloned().min_by(|a, b| (a - target).abs().partial_cmp(&(b - target).abs()).unwrap()).unwrap_or(0.0)
}

/// Approximate height (m) of a half level eta in the log-pressure coordinate.
pub fn height_of(eta: f64, scale_height: f64) -> f64 {
    if eta <= 0.0 {
        f64::INFINITY
    } else {
        -scale_height * eta.ln()
    }
}

/// Checks a list of half levels: from 0 (top) to 1 (ground), strictly increasing.
pub fn validate(eta_half: &[f64]) -> Result<(), String> {
    let n = eta_half.len();
    if n < 3 || n > 17 {
        return Err(format!("{} half levels: 2 to 16 layers expected", n));
    }
    if eta_half[0] != 0.0 || eta_half[n - 1] != 1.0 {
        return Err("half levels must start at 0 (top) and end at 1 (ground)".into());
    }
    if eta_half.windows(2).any(|w| w[1] <= w[0]) {
        return Err("half levels must increase strictly from the top to the ground".into());
    }
    Ok(())
}
