use std::{
    collections::VecDeque,
    sync::Mutex,
};

use rand::{Rng, seq::SliceRandom};

/// Pool of NPC display names. Loaded once from a text file (one name per
/// line); supports `take()`/`release()` semantics so a name is not reused
/// while it's in flight. Falls back to a random `Npc-XXXX` when empty.
pub struct NamePool {
    inner: Mutex<VecDeque<String>>,
}

impl NamePool {
    pub fn new(names: Vec<String>) -> Self {
        let mut shuffled = names;
        shuffled.shuffle(&mut rand::thread_rng());
        Self {
            inner: Mutex::new(VecDeque::from(shuffled)),
        }
    }

    pub fn from_file(path: &str) -> std::io::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let names: Vec<String> = raw
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        Ok(Self::new(names))
    }

    pub fn take(&self) -> String {
        if let Some(n) = self.inner.lock().unwrap().pop_front() {
            return n;
        }
        let n: u16 = rand::thread_rng().gen_range(1000..9999);
        format!("Npc-{n}")
    }

    pub fn release(&self, name: String) {
        if name.starts_with("Npc-") {
            return; // synthetic, don't recycle
        }
        self.inner.lock().unwrap().push_back(name);
    }
}
