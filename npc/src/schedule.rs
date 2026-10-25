//! Shift schedules: per-NPC grids of planetary hours mapping each hour of the
//! rotating rota to an [`Activity`], and each activity to the set of actions the
//! NPC may run during it. Stored in the database (imported from
//! `config/schedules.yaml`, edited from the admin panel) and hot-swapped through
//! [`ScheduleStore`].
//!
//! The schedule is a *gate layered on top of* the existing `action_profile` +
//! `can_run` scheduler: at each tick the runtime asks the schedule which actions
//! are allowed at the current [`PlanetTime`](crate::clock::PlanetTime), and the
//! [`ActionRegistry`](crate::actions::ActionRegistry) keeps only those.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use arc_swap::ArcSwap;
use serde::Deserialize;
use sqlx::PgPool;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::{clock::PlanetTime, error::Result};

/// Grid letters a pattern may use (see `config/schedules.yaml`). `l` (leisure)
/// has no behaviour yet and reads as off-shift.
pub const PATTERN_LETTERS: &str = "Ttra.l";

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
    pub schedules: BTreeMap<String, RawPattern>,
    /// Optional override of the code-default activity → action-name mapping.
    /// Keys: `work`, `transmission`, `meal`, `mandatory`, `off`.
    #[serde(default)]
    pub activity_actions: Option<HashMap<String, Vec<String>>>,
    /// Optional override of the always-allowed (ambient) actions.
    #[serde(default)]
    pub always_allowed: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RawPattern {
    pub days: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Check a pattern grid as the admin panel and the import accept it: 1 to 14
/// days, all the same width, only [`PATTERN_LETTERS`]. Stricter than the
/// runtime parser, which reads unknown letters as off-shift.
pub fn validate_pattern(name: &str, days: &[String]) -> std::result::Result<(), String> {
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        || name.is_empty()
        || name.len() > 64
    {
        return Err(format!("pattern name '{name}' must match [a-z0-9_]{{1,64}}"));
    }
    if days.is_empty() || days.len() > 14 {
        return Err(format!("pattern '{name}' must have 1 to 14 days, has {}", days.len()));
    }
    if let Some(bad) = days.iter().flat_map(|d| d.chars()).find(|c| !PATTERN_LETTERS.contains(*c)) {
        return Err(format!("pattern '{name}' uses '{bad}'; allowed letters: {PATTERN_LETTERS}"));
    }
    ShiftPattern::parse(name, days).map(|_| ()).map_err(|e| e.to_string())
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

    /// Build from database rows: `(name, days)` patterns and `(activity,
    /// actions)` mapping rows (activity `always` = ambient actions). Activities
    /// without a row keep their code default.
    pub fn from_parts(
        patterns: impl IntoIterator<Item = (String, Vec<String>)>,
        activity_actions: impl IntoIterator<Item = (String, Vec<String>)>,
    ) -> Self {
        let mut file = ScheduleFile::default();
        for (name, days) in patterns {
            file.schedules.insert(name, RawPattern { days, description: None });
        }
        let mut mapping = HashMap::new();
        for (activity, actions) in activity_actions {
            if activity == "always" {
                file.always_allowed = Some(actions);
            } else {
                mapping.insert(activity, actions);
            }
        }
        file.activity_actions = Some(mapping);
        Self::from_file(file)
    }

    /// Names of the known patterns, sorted.
    pub fn pattern_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.patterns.keys().cloned().collect();
        names.sort();
        names
    }

    /// The effective activity → actions mapping (`always` = ambient), for the
    /// admin panel.
    pub fn activity_actions(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = self
            .activity_actions
            .iter()
            .map(|(a, v)| (a.key().to_string(), v.clone()))
            .collect();
        out.insert("always".into(), self.always_allowed.clone());
        out
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

impl Activity {
    /// Key used in `activity_actions` (YAML and database).
    pub fn key(&self) -> &'static str {
        match self {
            Activity::Work => "work",
            Activity::Transmission => "transmission",
            Activity::Meal => "meal",
            Activity::Mandatory => "mandatory",
            Activity::Off => "off",
        }
    }
}

/// The shift schedules in force, swapped whole when an admin edits a pattern
/// or the activity mapping. Readers take a cheap snapshot (`load`) per tick;
/// `subscribe` wakes the NPCs waiting out an off-shift window.
pub struct ScheduleStore {
    current: ArcSwap<Schedule>,
    version: watch::Sender<u64>,
}

impl ScheduleStore {
    pub fn new(schedule: Schedule) -> Self {
        Self { current: ArcSwap::from_pointee(schedule), version: watch::Sender::new(0) }
    }

    pub fn load(&self) -> Arc<Schedule> {
        self.current.load_full()
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }

    pub fn replace(&self, schedule: Schedule) {
        self.current.store(Arc::new(schedule));
        self.version.send_modify(|v| *v += 1);
    }

    /// Rebuild from the database.
    pub async fn reload(&self, pool: &PgPool) -> Result<()> {
        let patterns = crate::db::patterns::list(pool).await?;
        let mapping = crate::db::patterns::list_activity_actions(pool).await?;
        let count = patterns.len();
        self.replace(Schedule::from_parts(
            patterns.into_iter().map(|p| (p.name, p.days)),
            mapping,
        ));
        info!(patterns = count, "shift schedules loaded");
        Ok(())
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
    fn from_parts_builds_patterns_and_mapping() {
        let s = Schedule::from_parts(
            [("p".to_string(), vec!["TTTTTaaarTTTTTaar........".to_string()])],
            [
                ("work".to_string(), vec!["drive_to_factory".to_string()]),
                ("always".to_string(), vec![]),
            ],
        );
        let work = s.allowed_actions_at("p", pt(0, 0)).unwrap();
        assert!(work.contains("drive_to_factory"));
        assert!(!work.contains("depot_store_item"), "row replaces the code default");
        assert!(!work.contains("detect_player_and_greet"), "empty `always` row clears ambient");
        // No `meal` row: code default kept.
        assert!(s.allowed_actions_at("p", pt(0, 8)).unwrap().contains("go_to_meal"));
        assert_eq!(s.pattern_names(), ["p"]);
    }

    #[test]
    fn validates_pattern_grids() {
        let ok = vec!["TTTTTaaarTTTTTaar........".to_string(); 5];
        assert!(validate_pattern("magasinier_1", &ok).is_ok());
        assert!(validate_pattern("Bad-Name", &ok).is_err());
        assert!(validate_pattern("p", &[]).is_err());
        assert!(validate_pattern("p", &["TTx".to_string()]).is_err());
        assert!(validate_pattern("p", &["TTT".to_string(), "TT".to_string()]).is_err());
    }

    #[test]
    fn store_swaps_and_notifies() {
        let store = ScheduleStore::new(Schedule::empty());
        let rx = store.subscribe();
        assert!(!store.load().has_pattern("magasinier_1"));
        store.replace(Schedule::from_yaml(YAML).unwrap());
        assert!(store.load().has_pattern("magasinier_1"));
        assert!(rx.has_changed().unwrap());
    }

    #[test]
    fn day_and_hour_wrap_defensively() {
        let s = Schedule::from_yaml(YAML).unwrap();
        // day 2 wraps to day 0 (pattern has 2 days), hour 25 wraps to hour 0.
        assert_eq!(s.activity_at("magasinier_1", pt(2, 25)), Some(Activity::Work));
    }
}
