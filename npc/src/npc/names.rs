//! Generated NPC names: a first name and a last name drawn from two lists
//! (`config/names/first.txt`, `last.txt`, one name per line, `#` comments).
//! Uniqueness is the database's job (`npcs_display_name_uq`); the caller
//! retries on a clash.

use rand::{Rng, seq::SliceRandom};

pub struct NameGenerator {
    first: Vec<String>,
    last: Vec<String>,
}

impl NameGenerator {
    pub fn new(first: Vec<String>, last: Vec<String>) -> Self {
        Self { first, last }
    }

    pub fn from_files(first: &str, last: &str) -> std::io::Result<Self> {
        Ok(Self::new(read_list(first)?, read_list(last)?))
    }

    /// `(first, last)`. Falls back to `Npc` / a number when a list is empty.
    pub fn generate(&self, rng: &mut impl Rng) -> (String, String) {
        let first = self.first.choose(rng).cloned().unwrap_or_else(|| "Npc".to_string());
        let last = self
            .last
            .choose(rng)
            .cloned()
            .unwrap_or_else(|| format!("{:04}", rng.gen_range(0..10_000)));
        (first, last)
    }

    /// How many distinct names the lists can produce.
    pub fn capacity(&self) -> usize {
        self.first.len().max(1) * self.last.len().max(1)
    }
}

fn read_list(path: &str) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

/// A fresh Keycloak login: `npc-` and 8 hex digits.
pub fn generate_login(rng: &mut impl Rng) -> String {
    format!("npc-{:08x}", rng.r#gen::<u32>())
}

#[cfg(test)]
mod tests {
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;

    #[test]
    fn draws_from_both_lists() {
        let g = NameGenerator::new(vec!["Jean".into()], vec!["Caisse".into(), "Deloin".into()]);
        let mut rng = StdRng::seed_from_u64(1);
        for _ in 0..10 {
            let (f, l) = g.generate(&mut rng);
            assert_eq!(f, "Jean");
            assert!(l == "Caisse" || l == "Deloin");
        }
        assert_eq!(g.capacity(), 2);
    }

    #[test]
    fn empty_lists_still_name() {
        let g = NameGenerator::new(vec![], vec![]);
        let (f, l) = g.generate(&mut StdRng::seed_from_u64(1));
        assert_eq!(f, "Npc");
        assert_eq!(l.len(), 4);
    }

    #[test]
    fn logins_are_lowercase_and_prefixed() {
        let login = generate_login(&mut StdRng::seed_from_u64(1));
        assert!(login.starts_with("npc-"));
        assert_eq!(login.len(), 12);
        assert_eq!(login, login.to_lowercase());
    }

    #[test]
    fn shipped_lists_are_readable() {
        let g = NameGenerator::from_files("config/names/first.txt", "config/names/last.txt").unwrap();
        assert!(g.capacity() >= 2_500, "lists too short for a few villages");
    }
}
