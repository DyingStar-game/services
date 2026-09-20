//! Shift schedules: per-NPC grids of planetary hours mapping each hour of the
//! rotating rota to an [`Activity`], and each activity to the set of actions the
//! NPC may run during it. Loaded from `config/schedules.yaml`.
//!
//! The schedule is a *gate layered on top of* the existing `action_profile` +
//! `can_run` scheduler: at each tick the runtime asks the schedule which actions
//! are allowed at the current [`PlanetTime`](crate::clock::PlanetTime), and the
//! [`ActionRegistry`](crate::actions::ActionRegistry) keeps only those.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use tracing::warn;

use crate::clock::PlanetTime;

/// What an NPC is meant to be doing during a given planetary hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activity {
    /// `T` travail — depot work.
    Work,
    /// `t` transmission — shift handoff (treated as work for MVP).
    Transmission,
    /// `r` repas — meal in the common zone.
    Meal,
    /// `a` activités obligatoires — medical / RH / cleaning / chores.
    Mandatory,
    /// blank / `.` — off-shift (home / sleep).
    Off,
}

impl Activity {
    /// Map a grid character to an activity. Unknown characters and whitespace
    /// are treated as [`Activity::Off`].
    pub fn from_char(c: char) -> Activity {
        match c {
            'T' => Activity::Work,
            't' => Activity::Transmission,
            'r' => Activity::Meal,
            'a' => Activity::Mandatory,
            _ => Activity::Off, // '.', ' ', anything else
        }
    }
}

/// A parsed shift pattern: one row of activities per rota-day.
#[derive(Debug, Clone)]
pub struct ShiftPattern {
    days: Vec<Vec<Activity>>,
}

impl ShiftPattern {
    fn parse(name: &str, rows: &[String]) -> anyhow::Result<Self> {
        if rows.is_empty() {
            anyhow::bail!("schedule '{name}' has no days");
        }
        let width = rows[0].chars().count();
        if width == 0 {
            anyhow::bail!("schedule '{name}' day 1 is empty");
        }
        let mut days = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let n = row.chars().count();
            if n != width {
                anyhow::bail!(
                    "schedule '{name}' day {} has {n} chars, expected {width} (all days must be the same length)",
                    i + 1
                );
            }
            days.push(row.chars().map(Activity::from_char).collect());
        }
        Ok(Self { days })
    }

    /// Activity at `(day, hour)`, indices wrapped defensively so an out-of-range
    /// day or hour can never panic.
    pub fn activity_at(&self, day: u32, hour: u32) -> Activity {
        let row = &self.days[(day as usize) % self.days.len()];
        row[(hour as usize) % row.len()]
    }

    /// Number of rota-days in this pattern.
    pub fn days(&self) -> usize {
        self.days.len()
    }

    /// Planetary hours per rota-day (the row width; SPECS uses 25).
    pub fn hours_per_day(&self) -> usize {
        self.days.first().map_or(0, |r| r.len())
    }
}

/// All schedules plus the activity → allowed-actions mapping.
pub struct Schedule {
    patterns: HashMap<String, ShiftPattern>,
    activity_actions: HashMap<Activity, Vec<String>>,
    /// Actions allowed in *every* activity (ambient behaviours, e.g. greeting).
    always_allowed: Vec<String>,
}

/// Wire format of `config/schedules.yaml`.
#[derive(Debug, Default, Deserialize)]
pub struct ScheduleFile {
    #[serde(default)]
    schedules: HashMap<String, RawPattern>,
    /// Optional override of the code-default activity → action-name mapping.
    /// Keys: `work`, `transmission`, `meal`, `mandatory`, `off`.
    #[serde(default)]
    activity_actions: Option<HashMap<String, Vec<String>>>,
    /// Optional override of the always-allowed (ambient) actions.
    #[serde(default)]
    always_allowed: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawPattern {
    days: Vec<String>,
}

impl Schedule {
    /// An empty schedule that gates nothing (every pattern lookup misses).
    pub fn empty() -> Self {
        Self {
            patterns: HashMap::new(),
            activity_actions: default_activity_actions(),
            always_allowed: default_always_allowed(),
        }
    }

    /// Parse a [`ScheduleFile`]. Individual malformed patterns are skipped with a
    /// warning rather than failing the whole load.
    pub fn from_file(file: ScheduleFile) -> Self {
        let mut patterns = HashMap::new();
        for (name, raw) in file.schedules {
            match ShiftPattern::parse(&name, &raw.days) {
                Ok(p) => {
                    patterns.insert(name, p);
                }
                Err(e) => warn!(schedule = %name, error = %e, "skipping malformed schedule"),
            }
        }

        let mut activity_actions = default_activity_actions();
        if let Some(overrides) = file.activity_actions {
            for (k, v) in overrides {
                if let Some(act) = parse_activity_key(&k) {
                    activity_actions.insert(act, v);
                } else {
                    warn!(key = %k, "unknown activity key in activity_actions; ignoring");
                }
            }
        }

        Self {
            patterns,
            activity_actions,
            always_allowed: file.always_allowed.unwrap_or_else(default_always_allowed),
        }
    }

    /// Parse the YAML text of a schedules file.
    pub fn from_yaml(text: &str) -> anyhow::Result<Self> {
        let file: ScheduleFile = serde_yaml::from_str(text)?;
        Ok(Self::from_file(file))
    }

    /// Whether a pattern by this name is known.
    pub fn has_pattern(&self, name: &str) -> bool {
        self.patterns.contains_key(name)
    }

    /// Current activity for `pattern` at `t`, or `None` if the pattern is
    /// unknown (caller should then not gate).
    pub fn activity_at(&self, pattern: &str, t: PlanetTime) -> Option<Activity> {
        self.patterns
            .get(pattern)
            .map(|p| p.activity_at(t.day, t.hour))
    }

    /// Whether `pattern` is on shift (Work or Transmission) at `t`. Unknown
    /// pattern counts as not working.
    pub fn is_working(&self, pattern: &str, t: PlanetTime) -> bool {
        matches!(
            self.activity_at(pattern, t),
            Some(Activity::Work | Activity::Transmission)
        )
    }

    /// Number of consecutive Work/Transmission planetary hours ending at `t`
    /// (inclusive), walking backwards through the rota with day/hour wrap. `0`
    /// when `t` is not a work hour or the pattern is unknown. Used to rank
    /// co-workers by shift-arrival order: the one on shift the longest (largest
    /// run length) arrived first. Capped at the full rota so an all-work pattern
    /// cannot loop forever.
    pub fn work_run_length(&self, pattern: &str, t: PlanetTime) -> u32 {
        let Some(p) = self.patterns.get(pattern) else {
            return 0;
        };
        let hpd = p.hours_per_day() as i64;
        let days = p.days() as i64;
        if hpd == 0 || days == 0 {
            return 0;
        }
        let total = hpd * days;
        let mut idx =
            ((t.day as i64 % days) * hpd + (t.hour as i64 % hpd)).rem_euclid(total);
        let mut count = 0u32;
        for _ in 0..total {
            let day = (idx / hpd) as u32;
            let hour = (idx % hpd) as u32;
            if matches!(
                p.activity_at(day, hour),
                Activity::Work | Activity::Transmission
            ) {
                count += 1;
            } else {
                break;
            }
            idx = (idx - 1).rem_euclid(total);
        }
        count
    }

    /// Set of action names allowed for `pattern` at `t`, or `None` if the
    /// pattern is unknown (caller should then allow all).
    pub fn allowed_actions_at(&self, pattern: &str, t: PlanetTime) -> Option<HashSet<String>> {
        let activity = self.activity_at(pattern, t)?;
        let mut set: HashSet<String> = self
            .activity_actions
            .get(&activity)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        set.extend(self.always_allowed.iter().cloned());
        Some(set)
    }
}

fn parse_activity_key(k: &str) -> Option<Activity> {
    match k.to_lowercase().as_str() {
        "work" | "t_upper" | "travail" => Some(Activity::Work),
        "transmission" => Some(Activity::Transmission),
        "meal" | "repas" => Some(Activity::Meal),
        "mandatory" | "activites" => Some(Activity::Mandatory),
        "off" => Some(Activity::Off),
        _ => None,
    }
}

/// Code-default mapping. `t` (transmission) shares `T`'s set for MVP so the box
/// flow keeps moving during a handoff.
fn default_activity_actions() -> HashMap<Activity, Vec<String>> {
    let work = vec![
        "going_to_work".to_string(),
        "depot_store_reception_to_temp_area".to_string(),
        "depot_store_item".to_string(),
        "reception_players".to_string(),
    ];
    let mut m = HashMap::new();
    m.insert(Activity::Work, work.clone());
    m.insert(Activity::Transmission, work);
    m.insert(Activity::Meal, vec!["go_to_meal".to_string()]);
    m.insert(Activity::Mandatory, vec!["mandatory_activity".to_string()]);
    m.insert(Activity::Off, vec!["go_home".to_string()]);
    m
}

fn default_always_allowed() -> Vec<String> {
    vec![
        "detect_player_and_greet".to_string(),
        // Eligible in every activity so a magasinier can hand its reception claim
        // back (release) the moment it stops working — meal, mandatory, or home —
        // not only during Work. `can_run` is edge-triggered, so it stays inert
        // unless a claim/line/release is actually owed.
        "depot_reception_coordinator".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(day: u32, hour: u32) -> PlanetTime {
        PlanetTime {
            day,
            hour,
            hour_f: hour as f64,
        }
    }

    // Day 0: hour 0-4 = T, 5-7 = a, 8 = r, 9-13 = T, 14-15 = a, 16 = r, 17-24 = off.
    // Day 1: hour 0-1 = r, rest off. Both lines exactly 25 chars.
    const YAML: &str = r#"
schedules:
  magasinier_1:
    days:
      - "TTTTTaaarTTTTTaar........"
      - "rr......................."
"#;

    #[test]
    fn parses_and_reads_activities() {
        let s = Schedule::from_yaml(YAML).unwrap();
        assert!(s.has_pattern("magasinier_1"));
        assert_eq!(s.activity_at("magasinier_1", pt(0, 0)), Some(Activity::Work)); // 'T'
        assert_eq!(s.activity_at("magasinier_1", pt(0, 5)), Some(Activity::Mandatory)); // 'a'
        assert_eq!(s.activity_at("magasinier_1", pt(0, 8)), Some(Activity::Meal)); // 'r'
        assert_eq!(s.activity_at("magasinier_1", pt(0, 24)), Some(Activity::Off)); // '.'
        assert_eq!(s.activity_at("magasinier_1", pt(1, 0)), Some(Activity::Meal)); // day 2 'r'
    }

    #[test]
    fn unknown_pattern_returns_none() {
        let s = Schedule::from_yaml(YAML).unwrap();
        assert_eq!(s.activity_at("nope", pt(0, 0)), None);
        assert!(s.allowed_actions_at("nope", pt(0, 0)).is_none());
    }

    #[test]
    fn allowed_actions_include_activity_set_plus_ambient() {
        let s = Schedule::from_yaml(YAML).unwrap();
        let work = s.allowed_actions_at("magasinier_1", pt(0, 0)).unwrap();
        assert!(work.contains("depot_store_item"));
        assert!(work.contains("going_to_work"));
        assert!(work.contains("detect_player_and_greet")); // ambient
        assert!(!work.contains("go_to_meal"));

        let meal = s.allowed_actions_at("magasinier_1", pt(0, 8)).unwrap();
        assert!(meal.contains("go_to_meal"));
        assert!(meal.contains("detect_player_and_greet"));
        assert!(!meal.contains("depot_store_item"));

        let off = s.allowed_actions_at("magasinier_1", pt(0, 24)).unwrap();
        assert!(off.contains("go_home"));
        assert!(!off.contains("depot_store_item"));
    }

    #[test]
    fn rejects_ragged_day_lengths() {
        let bad = r#"
schedules:
  x:
    days:
      - "TTTT"
      - "TT"
"#;
        // Malformed pattern is skipped, so it simply isn't present.
        let s = Schedule::from_yaml(bad).unwrap();
        assert!(!s.has_pattern("x"));
    }

    #[test]
    fn day_and_hour_wrap_defensively() {
        let s = Schedule::from_yaml(YAML).unwrap();
        // day 2 wraps to day 0 (pattern has 2 days), hour 25 wraps to hour 0.
        assert_eq!(s.activity_at("magasinier_1", pt(2, 25)), Some(Activity::Work));
    }
}
