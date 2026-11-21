//! Generated NPC names: a first name and a last name from the locale lists
//! of the `fake` crate, both from the same locale so they sound like one
//! person. Latin-script locales only: the display name travels to Horizon
//! and Godot. French alone gives about 880 000 pairs. Uniqueness is the
//! database's job (`npcs_display_name_uq`); the caller retries on a clash.

use fake::{
    Fake,
    faker::name::raw::{FirstName, LastName},
    locales::{DE_DE, EN, FR_FR, IT_IT, PT_BR},
    rand::RngExt,
};
use rand::Rng;

/// The locales a name is drawn from, with their weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Locale {
    Fr,
    En,
    De,
    It,
    PtBr,
}

const LOCALES: &[(Locale, u32)] = &[
    (Locale::Fr, 4),
    (Locale::En, 3),
    (Locale::De, 1),
    (Locale::It, 1),
    (Locale::PtBr, 1),
];

#[derive(Default)]
pub struct NameGenerator;

impl NameGenerator {
    pub fn new() -> Self {
        Self
    }

    /// `(first, last)`.
    pub fn generate(&self) -> (String, String) {
        self.generate_with(&mut fake::rand::rng())
    }

    pub fn generate_with<R: RngExt + ?Sized>(&self, rng: &mut R) -> (String, String) {
        let total: u32 = LOCALES.iter().map(|(_, w)| w).sum();
        let mut pick = rng.random_range(0..total);
        let locale = LOCALES
            .iter()
            .find(|(_, w)| {
                if pick < *w {
                    true
                } else {
                    pick -= w;
                    false
                }
            })
            .map_or(Locale::Fr, |(l, _)| *l);
        let (first, last): (String, String) = match locale {
            Locale::Fr => (FirstName(FR_FR).fake_with_rng(rng), LastName(FR_FR).fake_with_rng(rng)),
            Locale::En => (FirstName(EN).fake_with_rng(rng), LastName(EN).fake_with_rng(rng)),
            Locale::De => (FirstName(DE_DE).fake_with_rng(rng), LastName(DE_DE).fake_with_rng(rng)),
            Locale::It => (FirstName(IT_IT).fake_with_rng(rng), LastName(IT_IT).fake_with_rng(rng)),
            Locale::PtBr => (FirstName(PT_BR).fake_with_rng(rng), LastName(PT_BR).fake_with_rng(rng)),
        };
        // Some entries of the lists carry stray whitespace (`"Ricci\t"`).
        (first.trim().to_string(), last.trim().to_string())
    }
}

/// A fresh Keycloak login: `npc-` and 8 hex digits.
pub fn generate_login(rng: &mut impl Rng) -> String {
    format!("npc-{:08x}", rng.r#gen::<u32>())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use fake::rand::{SeedableRng as _, rngs::StdRng as FakeRng};
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;

    #[test]
    fn names_are_non_empty_and_trimmed() {
        let g = NameGenerator::new();
        let mut rng = FakeRng::seed_from_u64(1);
        for _ in 0..500 {
            let (f, l) = g.generate_with(&mut rng);
            assert!(!f.is_empty() && !l.is_empty());
            assert_eq!(f, f.trim());
            assert_eq!(l, l.trim());
        }
    }

    #[test]
    fn two_thousand_names_rarely_clash() {
        // 2 000 NPCs need well under 10 retries each (provision::create_one).
        let g = NameGenerator::new();
        let mut rng = FakeRng::seed_from_u64(7);
        let names: HashSet<String> = (0..2_000)
            .map(|_| {
                let (f, l) = g.generate_with(&mut rng);
                format!("{f} {l}").to_lowercase()
            })
            .collect();
        assert!(names.len() >= 1_990, "only {} distinct names out of 2000", names.len());
    }

    #[test]
    fn logins_are_lowercase_and_prefixed() {
        let login = generate_login(&mut StdRng::seed_from_u64(1));
        assert!(login.starts_with("npc-"));
        assert_eq!(login.len(), 12);
        assert_eq!(login, login.to_lowercase());
    }
}
