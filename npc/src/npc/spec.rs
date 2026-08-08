use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{actions::ActionTiming, horizon::messages::Vec3};

/// Per-NPC override of an action's code-default [`ActionTiming`]. Any field left
/// unset keeps the action's own default. Millisecond fields are converted to
/// `Duration`; `Some(0)` for a duration means "no restriction" (clears it).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimingOverride {
    #[serde(default)]
    pub once: Option<bool>,
    #[serde(default)]
    pub cooldown_ms: Option<u64>,
    #[serde(default)]
    pub min_interval_ms: Option<u64>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl TimingOverride {
    /// Apply this override on top of an action's default timing.
    pub fn apply(&self, mut base: ActionTiming) -> ActionTiming {
        if let Some(once) = self.once {
            base.once = once;
        }
        if let Some(ms) = self.cooldown_ms {
            base.cooldown = (ms > 0).then(|| Duration::from_millis(ms));
        }
        if let Some(ms) = self.min_interval_ms {
            base.min_interval = (ms > 0).then(|| Duration::from_millis(ms));
        }
        if let Some(ms) = self.timeout_ms {
            base.timeout = (ms > 0).then(|| Duration::from_millis(ms));
        }
        base
    }
}

/// Where an NPC goes to work. NPCs spawn in their habitation; if this is set,
/// the NPC is sent to this place as soon as its Horizon session is up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceWhereWork {
    /// UUID of the work place, sent as the `uuid` of the go-to-position action.
    pub uuid: String,

    /// Planet the place sits on. Unused for now; `null` in the roster.
    #[serde(default)]
    pub planet: Option<String>,

    /// Target position within the place.
    pub position: Vec3,
}

/// Per-NPC configuration of the `depot_store_reception_to_temp_area` action.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DepotStoreReceptionToTempAreaConfig {
    /// Scene file names (e.g. `hauling_box.tscn`) accepted on the reception
    /// counter. Empty disables the action.
    #[serde(default)]
    pub boxes_type_allowed: Vec<String>,
}

/// Per-NPC configuration of the `depot_store_item` action.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DepotStoreItemConfig {
    /// Metres the NPC stands in front of the shelf face when dropping a box
    /// into a slot. `None` = the code default (1.0 m).
    #[serde(default)]
    pub stand_distance_m: Option<f64>,
}

/// Description of a single NPC, supplied either by the YAML roster or by the
/// admin REST endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpcSpec {
    /// Display name. If `None`, one is drawn from the name pool at spawn time.
    #[serde(default)]
    pub name: Option<String>,

    /// Stable UUID for this NPC. Used as `sub` in the generated JWT and as the
    /// player identity on Horizon.
    pub uuid: String,

    /// Names of actions (as exposed by their `Action::name()`) the NPC is
    /// allowed to run. Empty = no actions (idle forever).
    #[serde(default)]
    pub action_profile: Vec<String>,

    /// Where the NPC works. `None` = stays in its habitation.
    #[serde(default)]
    pub place_where_work: Option<PlaceWhereWork>,

    /// Per-action timing overrides, keyed by `Action::name()`. Actions not
    /// listed here use their code-default [`ActionTiming`]. Lets designers tune
    /// cadence per NPC without recompiling.
    #[serde(default)]
    pub action_timing: HashMap<String, TimingOverride>,

    /// Config of the `depot_store_reception_to_temp_area` action. `None` (or an
    /// empty allow-list) disables it even when listed in `action_profile`.
    #[serde(default)]
    pub depot_store_reception_to_temp_area: Option<DepotStoreReceptionToTempAreaConfig>,

    /// Config of the `depot_store_item` action. Optional: unlike the reception
    /// action, everything has a sane default — only `action_profile` gates it.
    #[serde(default)]
    pub depot_store_item: Option<DepotStoreItemConfig>,
}

/// YAML schema for `config/npcs.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RosterFile {
    #[serde(default)]
    pub npcs: Vec<NpcSpec>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_place_where_work_from_roster() {
        let roster: RosterFile = serde_yaml::from_str(
            r#"
npcs:
  - name: Alice
    uuid: "b4a36a46-4424-4f66-bd14-caaf8be287cb"
    action_profile:
      - going_to_work
    place_where_work: {
      uuid: "ed20bda3-f6f3-4053-b9de-968f73ebc44c",
      planet: null,
      position: {x: 50.858, y: 1.0, z: -37.942}
    }
"#,
        )
        .unwrap();
        let place = roster.npcs[0].place_where_work.as_ref().unwrap();
        assert_eq!(place.uuid, "ed20bda3-f6f3-4053-b9de-968f73ebc44c");
        assert!(place.planet.is_none());
        assert_eq!(place.position.z, -37.942);
    }

    #[test]
    fn parses_action_timing_overrides() {
        let roster: RosterFile = serde_yaml::from_str(
            r#"
npcs:
  - uuid: "b4a36a46-4424-4f66-bd14-caaf8be287cb"
    action_profile:
      - detect_player_and_greet
    action_timing:
      detect_player_and_greet:
        timeout_ms: 5000
        cooldown_ms: 2000
"#,
        )
        .unwrap();
        let ov = &roster.npcs[0].action_timing["detect_player_and_greet"];
        assert_eq!(ov.timeout_ms, Some(5000));
        assert_eq!(ov.cooldown_ms, Some(2000));
    }

    #[test]
    fn override_applies_over_defaults_and_clears_on_zero() {
        let base = ActionTiming {
            once: false,
            cooldown: Some(Duration::from_secs(60)),
            min_interval: None,
            timeout: None,
        };
        let ov = TimingOverride {
            once: Some(true),
            cooldown_ms: Some(0), // clears the 60s default
            min_interval_ms: None,
            timeout_ms: Some(3000),
        };
        let eff = ov.apply(base);
        assert!(eff.once);
        assert_eq!(eff.cooldown, None);
        assert_eq!(eff.min_interval, None);
        assert_eq!(eff.timeout, Some(Duration::from_millis(3000)));
    }

    #[test]
    fn parses_depot_store_reception_config() {
        let roster: RosterFile = serde_yaml::from_str(
            r#"
npcs:
  - uuid: "b4a36a46-4424-4f66-bd14-caaf8be287cb"
    action_profile:
      - depot_store_reception_to_temp_area
    depot_store_reception_to_temp_area: {
      boxes_type_allowed: ["hauling_box.tscn"]
    }
"#,
        )
        .unwrap();
        let cfg = roster.npcs[0]
            .depot_store_reception_to_temp_area
            .as_ref()
            .unwrap();
        assert_eq!(cfg.boxes_type_allowed, vec!["hauling_box.tscn"]);
    }

    #[test]
    fn parses_depot_store_item_config() {
        let roster: RosterFile = serde_yaml::from_str(
            r#"
npcs:
  - uuid: "b4a36a46-4424-4f66-bd14-caaf8be287cb"
    action_profile:
      - depot_store_item
    depot_store_item: {
      stand_distance_m: 1.5
    }
"#,
        )
        .unwrap();
        let cfg = roster.npcs[0].depot_store_item.as_ref().unwrap();
        assert_eq!(cfg.stand_distance_m, Some(1.5));

        // The config block itself is optional.
        let roster: RosterFile = serde_yaml::from_str(
            r#"
npcs:
  - uuid: "b4a36a46-4424-4f66-bd14-caaf8be287cb"
    action_profile:
      - depot_store_item
"#,
        )
        .unwrap();
        assert!(roster.npcs[0].depot_store_item.is_none());
    }

    #[test]
    fn action_timing_defaults_to_empty() {
        let roster: RosterFile = serde_yaml::from_str(
            r#"
npcs:
  - uuid: "b4a36a46-4424-4f66-bd14-caaf8be287cb"
"#,
        )
        .unwrap();
        assert!(roster.npcs[0].action_timing.is_empty());
    }

    #[test]
    fn place_where_work_is_optional() {
        let roster: RosterFile = serde_yaml::from_str(
            r#"
npcs:
  - uuid: "b4a36a46-4424-4f66-bd14-caaf8be287cb"
"#,
        )
        .unwrap();
        assert!(roster.npcs[0].place_where_work.is_none());
    }
}
