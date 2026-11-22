//! Deterministic hashing: every discrete decision of the weather (does a pressure cell exist in this
//! slot at this epoch, how strong, which way) is drawn from an INTEGER hash of (seed, slot, epoch),
//! never from a float or a random generator. Two machines, or the service and the game's twin
//! (`weather_hash.gd`), then agree bit for bit, whatever their `libm`.
//!
//! SplitMix64 — small, fast, well distributed, and trivially re-implemented in GDScript on 64-bit
//! integers (GDScript `int` is i64; the wrapping arithmetic is the same).

/// Mix a seed with up to three integer coordinates into a 64-bit hash.
pub fn mix(seed: u64, a: i64, b: i64, c: i64) -> u64 {
    let mut h = seed;
    h = splitmix(h ^ (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    h = splitmix(h ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    h = splitmix(h ^ (c as u64).wrapping_mul(0x1656_67B1_9E37_79F9));
    h
}

/// One SplitMix64 step.
pub fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A hash as a float in [0, 1): the top 53 bits, so the value is exactly representable and the
/// same on every platform.
pub fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// A hash as a float in [-1, 1).
pub fn signed_unit(h: u64) -> f64 {
    unit(h) * 2.0 - 1.0
}

/// A seed from a body's name, so the data may name a seed or leave it to the name.
pub fn seed_from_name(name: &str) -> u64 {
    let mut h: u64 = 0xDEAD_BEEF_CAFE_F00D;
    for b in name.bytes() {
        h = splitmix(h ^ b as u64);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden values, computed by an independent Python implementation (2026-10-06): the game's
    /// weather_hash.gd must return exactly these.
    #[test]
    fn golden_vectors() {
        assert_eq!(splitmix(0), 0xE220_A839_7B1D_CDAF);
        assert_eq!(mix(1, 2, 3, 4), 0x3558_1A25_9136_820F);
        assert_eq!(mix(42, -7, 1_000_000, 0), 0x5F74_FF71_468F_CA65);
        assert_eq!(mix(20_261_006, 0, 0, 0), 0xB9EB_485B_B4FF_65E4);
        assert!((unit(mix(1, 2, 3, 4)) - 0.208_376).abs() < 1e-6);
        assert!((unit(mix(20_261_006, 0, 0, 0)) - 0.726_246).abs() < 1e-6);
    }

    #[test]
    fn unit_is_in_range_and_flat() {
        let n = 100_000;
        let mut buckets = [0u32; 10];
        for i in 0..n {
            let u = unit(mix(7, i, 0, 0));
            assert!((0.0..1.0).contains(&u));
            buckets[(u * 10.0) as usize] += 1;
        }
        for b in buckets {
            // 10 000 expected; a 3 % tolerance is ~10 sigma for a flat source.
            assert!((b as f64 - n as f64 / 10.0).abs() < 300.0, "{:?}", buckets);
        }
    }

    #[test]
    fn different_coordinates_differ() {
        assert_ne!(mix(1, 0, 0, 0), mix(1, 1, 0, 0));
        assert_ne!(mix(1, 0, 0, 0), mix(2, 0, 0, 0));
        assert_ne!(mix(1, 0, 1, 0), mix(1, 0, 0, 1));
    }
}
