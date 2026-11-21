//! Import of `config/schedules.yaml` into the database: every pattern and the
//! activity → actions mapping. Runs on first boot (empty table) and on demand
//! from the admin panel or the CLI (`npc import-schedules`).

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::{
    db::patterns,
    error::Result,
    schedule::{ScheduleFile, validate_pattern},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    /// Only add what is missing: edits made from the admin panel are kept.
    #[default]
    Missing,
    /// Replace existing patterns and mapping rows with the file's.
    Overwrite,
}

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub skipped: Vec<String>,
    /// `name: reason` of each pattern or mapping row that was refused.
    pub errors: Vec<String>,
}

impl ImportReport {
    pub fn changed(&self) -> bool {
        !self.created.is_empty() || !self.updated.is_empty()
    }
}

const ACTIVITY_KEYS: [&str; 5] = ["work", "transmission", "meal", "mandatory", "off"];

/// Parse `yaml` (the schedules file format) and write it to the database.
pub async fn import_schedules(pool: &PgPool, yaml: &str, mode: ImportMode, by: Option<&str>) -> Result<ImportReport> {
    let file: ScheduleFile = serde_yaml::from_str(yaml)?;
    let mut report = ImportReport::default();

    for (name, raw) in &file.schedules {
        if let Err(e) = validate_pattern(name, &raw.days) {
            report.errors.push(e);
            continue;
        }
        let description = raw.description.as_deref();
        match mode {
            ImportMode::Missing => {
                if patterns::insert_missing(pool, name, description, &raw.days, by).await? {
                    report.created.push(name.clone());
                } else {
                    report.skipped.push(name.clone());
                }
            }
            ImportMode::Overwrite => {
                if patterns::upsert(pool, name, description, &raw.days, "import", by).await? {
                    report.created.push(name.clone());
                } else {
                    report.updated.push(name.clone());
                }
            }
        }
    }

    let mut mapping: Vec<(String, Vec<String>)> = Vec::new();
    for (key, actions) in file.activity_actions.into_iter().flatten() {
        let key = key.to_lowercase();
        if ACTIVITY_KEYS.contains(&key.as_str()) {
            mapping.push((key, actions));
        } else {
            report.errors.push(format!("activity_actions.{key}: unknown activity"));
        }
    }
    if let Some(always) = file.always_allowed {
        mapping.push(("always".into(), always));
    }
    for (activity, actions) in mapping {
        let label = format!("activity_actions.{activity}");
        let written = match mode {
            ImportMode::Missing => {
                patterns::insert_missing_activity_actions(pool, &activity, &actions, by).await?.then_some(true)
            }
            ImportMode::Overwrite => Some(patterns::upsert_activity_actions(pool, &activity, &actions, by).await?),
        };
        match written {
            Some(true) => report.created.push(label),
            Some(false) => report.updated.push(label),
            None => report.skipped.push(label),
        }
    }
    Ok(report)
}
