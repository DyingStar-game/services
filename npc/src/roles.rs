//! Role templates (`config/roles.yaml`): what an NPC becomes when it is
//! assigned automatically at its first spawn — one of the five storekeepers of
//! its village, or a villager.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::types::Json;
use uuid::Uuid;

use crate::{
    db::npcs::NpcRow,
    error::{NpcError, Result},
    horizon::messages::Vec3,
};

#[derive(Debug, Clone, Deserialize)]
pub struct RoleTemplates {
    pub storekeeper: StorekeeperTemplate,
    pub villager: VillagerTemplate,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorekeeperTemplate {
    pub slots: u8,
    /// Pattern name; `{slot}` is replaced by 1..=slots.
    pub schedule: String,
    pub action_profile: Vec<String>,
    #[serde(default)]
    pub action_config: Value,
    /// Depot-local positions per slot, keyed by depot scene (`default` otherwise).
    pub layouts: HashMap<String, SlotLayout>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlotLayout {
    pub work: Vec<Vec3>,
    #[serde(default)]
    pub meal: Vec<Vec3>,
    #[serde(default)]
    pub mandatory: Vec<Vec3>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VillagerTemplate {
    #[serde(default)]
    pub schedule: Option<String>,
    pub action_profile: Vec<String>,
    #[serde(default)]
    pub action_config: Value,
}

/// The depot a storekeeper works at.
pub struct Depot<'a> {
    pub id: Uuid,
    pub scenename: Option<&'a str>,
    pub planet: &'a str,
}

impl RoleTemplates {
    pub fn load(path: &str) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let t: Self = serde_yaml::from_str(&text)?;
        if !t.storekeeper.layouts.contains_key("default") {
            return Err(NpcError::Config(format!("{path}: storekeeper.layouts needs a `default` entry")));
        }
        for (scene, l) in &t.storekeeper.layouts {
            if l.work.len() < t.storekeeper.slots as usize {
                return Err(NpcError::Config(format!(
                    "{path}: layout '{scene}' has {} work positions for {} slots",
                    l.work.len(),
                    t.storekeeper.slots
                )));
            }
        }
        Ok(t)
    }

    pub fn storekeeper_slots(&self) -> u8 {
        self.storekeeper.slots
    }

    /// Pattern of storekeeper `slot`.
    pub fn storekeeper_schedule(&self, slot: u8) -> String {
        self.storekeeper.schedule.replace("{slot}", &slot.to_string())
    }

    /// Turn `row` into storekeeper `slot` of `village`, working at `depot`.
    /// `schedule` is the pattern to use (`None` when it does not exist).
    pub fn apply_storekeeper(
        &self,
        row: &mut NpcRow,
        village: &str,
        slot: u8,
        depot: &Depot<'_>,
        schedule: Option<String>,
    ) {
        let t = &self.storekeeper;
        let layout = depot
            .scenename
            .and_then(|s| t.layouts.get(s).or_else(|| t.layouts.get(file_name(s))))
            .unwrap_or_else(|| &t.layouts["default"]);
        let i = (slot as usize).saturating_sub(1);
        let place = |positions: &Vec<Vec3>| {
            positions.get(i).map(|p| {
                Json(json!({ "uuid": depot.id.to_string(), "planet": depot.planet, "position": p }))
            })
        };
        row.role = "storekeeper".into();
        row.village_code = Some(village.to_string());
        row.storekeeper_slot = Some(slot as i16);
        row.schedule_pattern = schedule;
        row.action_profile = t.action_profile.clone();
        row.place_where_work = place(&layout.work);
        row.meal_place = place(&layout.meal);
        row.mandatory_place = place(&layout.mandatory);
        row.home = None;
        row.action_config = Json(object_or_empty(&t.action_config));
    }

    /// Turn `row` into a villager (of `village` when known).
    pub fn apply_villager(&self, row: &mut NpcRow, village: Option<&str>, schedule: Option<String>) {
        let t = &self.villager;
        row.role = "villager".into();
        row.village_code = village.map(str::to_string);
        row.storekeeper_slot = None;
        row.schedule_pattern = schedule;
        row.action_profile = t.action_profile.clone();
        row.place_where_work = None;
        row.meal_place = None;
        row.mandatory_place = None;
        row.home = None;
        row.action_config = Json(object_or_empty(&t.action_config));
    }
}

fn object_or_empty(v: &Value) -> Value {
    if v.is_object() { v.clone() } else { json!({}) }
}

/// `res://scenes/depot/cargo_depot.tscn` → `cargo_depot.tscn`.
fn file_name(scene: &str) -> &str {
    scene.rsplit('/').next().unwrap_or(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn templates() -> RoleTemplates {
        RoleTemplates::load("config/roles.yaml").unwrap()
    }

    #[test]
    fn shipped_templates_load() {
        let t = templates();
        assert_eq!(t.storekeeper_slots(), 5);
        assert_eq!(t.storekeeper_schedule(3), "magasinier_3");
    }

    #[test]
    fn storekeeper_gets_slot_positions_on_the_depot() {
        let t = templates();
        let mut row = crate::db::npcs::tests::row();
        let depot_id = Uuid::new_v4();
        let depot = Depot { id: depot_id, scenename: Some("res://x/unknown.tscn"), planet: "tarsis_3" };
        t.apply_storekeeper(&mut row, "M0012", 2, &depot, Some("magasinier_2".into()));
        assert_eq!(row.role, "storekeeper");
        assert_eq!(row.storekeeper_slot, Some(2));
        assert_eq!(row.village_code.as_deref(), Some("M0012"));
        let spec = row.to_spec().unwrap();
        let work = spec.place_where_work.unwrap();
        assert_eq!(work.uuid, depot_id.to_string());
        assert_eq!(work.planet.as_deref(), Some("tarsis_3"));
        assert_eq!(work.position.z, 6.913);
        assert_eq!(spec.meal_place.unwrap().position.z, 88.602);
        assert_eq!(spec.schedule.as_deref(), Some("magasinier_2"));
        assert!(spec.action_profile.contains(&"depot_store_item".to_string()));
        assert_eq!(
            spec.depot_store_reception_to_temp_area.unwrap().boxes_type_allowed,
            ["hauling_box.tscn"]
        );
    }

    #[test]
    fn villager_clears_storekeeper_fields() {
        let t = templates();
        let mut row = crate::db::npcs::tests::row();
        let depot = Depot { id: Uuid::new_v4(), scenename: None, planet: "tarsis_3" };
        t.apply_storekeeper(&mut row, "M0012", 1, &depot, None);
        t.apply_villager(&mut row, Some("M0012"), None);
        assert_eq!(row.role, "villager");
        assert!(row.storekeeper_slot.is_none());
        let spec = row.to_spec().unwrap();
        assert!(spec.place_where_work.is_none());
        assert!(spec.take_a_walk.is_some());
        assert!(spec.depot_store_item.is_none());
    }
}
