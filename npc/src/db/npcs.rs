use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Postgres, Transaction, types::Json};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    error::{NpcError, Result},
    npc::spec::NpcSpec,
};

/// Per-action config blocks of [`NpcSpec`] stored in `npcs.action_config`.
pub const ACTION_CONFIG_KEYS: [&str; 4] = [
    "depot_store_reception_to_temp_area",
    "depot_store_item",
    "take_a_walk",
    "drive_to_factory",
];

pub const ROLES: [&str; 4] = ["unassigned", "storekeeper", "villager", "custom"];

/// One NPC as stored. The sealed password never leaves the service.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct NpcRow {
    pub id: Uuid,
    pub login: String,
    #[serde(skip)]
    pub password_enc: Vec<u8>,
    #[serde(skip)]
    pub password_key_id: i16,
    pub first_name: String,
    pub last_name: String,
    pub display_name: String,
    pub enabled: bool,
    pub role: String,
    pub role_locked: bool,
    pub village_code: Option<String>,
    pub storekeeper_slot: Option<i16>,
    pub schedule_pattern: Option<String>,
    pub action_profile: Vec<String>,
    pub place_where_work: Option<Json<Value>>,
    pub home: Option<Json<Value>>,
    pub meal_place: Option<Json<Value>>,
    pub mandatory_place: Option<Json<Value>>,
    pub action_timing: Json<Value>,
    pub action_config: Json<Value>,
    pub kc_state: String,
    pub kc_error: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub first_spawn_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_seen_at: Option<OffsetDateTime>,
    pub notes: Option<String>,
    pub created_by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

const COLS: &str = "id, login, password_enc, password_key_id, first_name, last_name, display_name,
    enabled, role, role_locked, village_code, storekeeper_slot, schedule_pattern, action_profile,
    place_where_work, home, meal_place, mandatory_place, action_timing, action_config,
    kc_state, kc_error, first_spawn_at, last_seen_at, notes, created_by, created_at, updated_at";

impl NpcRow {
    /// The runtime spec of this NPC: the row's columns, with the per-action
    /// config blocks of `action_config` merged at the top level. Fails on
    /// malformed JSON, so writes validate through it before saving.
    pub fn to_spec(&self) -> Result<NpcSpec> {
        let mut obj = match &self.action_config.0 {
            Value::Object(m) => m.clone(),
            Value::Null => Map::new(),
            _ => return Err(NpcError::BadRequest("actionConfig must be an object".into())),
        };
        if let Some(k) = obj.keys().find(|k| !ACTION_CONFIG_KEYS.contains(&k.as_str())) {
            return Err(NpcError::BadRequest(format!(
                "unknown actionConfig key '{k}' (expected one of {ACTION_CONFIG_KEYS:?})"
            )));
        }
        let place = |p: &Option<Json<Value>>| p.as_ref().map(|j| j.0.clone()).unwrap_or(Value::Null);
        obj.insert("name".into(), json!(self.display_name));
        obj.insert("uuid".into(), json!(self.id.to_string()));
        obj.insert("action_profile".into(), json!(self.action_profile));
        obj.insert("schedule".into(), json!(self.schedule_pattern));
        obj.insert("place_where_work".into(), place(&self.place_where_work));
        obj.insert("home".into(), place(&self.home));
        obj.insert("meal_place".into(), place(&self.meal_place));
        obj.insert("mandatory_place".into(), place(&self.mandatory_place));
        obj.insert("action_timing".into(), self.action_timing.0.clone());
        serde_json::from_value(Value::Object(obj))
            .map_err(|e| NpcError::BadRequest(format!("invalid NPC configuration: {e}")))
    }

    /// The automatic assignment (storekeeper / villager) has yet to run.
    pub fn assignment_pending(&self) -> bool {
        self.role == "unassigned" && !self.role_locked
    }
}

/// A row to insert (Keycloak account already created).
pub struct NewNpc {
    pub id: Uuid,
    pub login: String,
    pub password_enc: Vec<u8>,
    pub first_name: String,
    pub last_name: String,
    pub display_name: String,
    pub enabled: bool,
    pub role: String,
    pub role_locked: bool,
    pub village_code: Option<String>,
    pub schedule_pattern: Option<String>,
    pub action_profile: Vec<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcFilter {
    pub village: Option<String>,
    pub role: Option<String>,
    pub enabled: Option<bool>,
    /// Matches display name or login (case-insensitive substring).
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list(pool: &PgPool, f: &NpcFilter) -> Result<(Vec<NpcRow>, i64)> {
    let filter = "($1::TEXT IS NULL OR village_code = $1)
        AND ($2::TEXT IS NULL OR role = $2)
        AND ($3::BOOLEAN IS NULL OR enabled = $3)
        AND ($4::TEXT IS NULL OR display_name ILIKE '%' || $4 || '%' OR login ILIKE '%' || $4 || '%')";
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM npcs WHERE {filter}"))
        .bind(&f.village)
        .bind(&f.role)
        .bind(f.enabled)
        .bind(&f.q)
        .fetch_one(pool)
        .await?;
    let rows = sqlx::query_as(&format!(
        "SELECT {COLS} FROM npcs WHERE {filter} ORDER BY created_at, id LIMIT $5 OFFSET $6"
    ))
    .bind(&f.village)
    .bind(&f.role)
    .bind(f.enabled)
    .bind(&f.q)
    .bind(f.limit.unwrap_or(100).clamp(1, 1000))
    .bind(f.offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await?;
    Ok((rows, total))
}

/// Every NPC, oldest first (the reconciler's view).
pub async fn all(pool: &PgPool) -> Result<Vec<NpcRow>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM npcs ORDER BY created_at, id"))
        .fetch_all(pool)
        .await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<NpcRow>> {
    Ok(sqlx::query_as(&format!("SELECT {COLS} FROM npcs WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn require(pool: &PgPool, id: Uuid) -> Result<NpcRow> {
    get(pool, id).await?.ok_or_else(|| NpcError::NotFound(format!("NPC {id}")))
}

pub async fn display_name_taken(pool: &PgPool, name: &str) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM npcs WHERE lower(display_name) = lower($1))")
        .bind(name)
        .fetch_one(pool)
        .await?)
}

pub async fn insert(pool: &PgPool, n: &NewNpc) -> Result<NpcRow> {
    Ok(sqlx::query_as(&format!(
        "INSERT INTO npcs (id, login, password_enc, password_key_id, first_name, last_name, display_name,
             enabled, role, role_locked, village_code, schedule_pattern, action_profile, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         RETURNING {COLS}"
    ))
    .bind(n.id)
    .bind(&n.login)
    .bind(&n.password_enc)
    .bind(crate::crypto::KEY_ID)
    .bind(&n.first_name)
    .bind(&n.last_name)
    .bind(&n.display_name)
    .bind(n.enabled)
    .bind(&n.role)
    .bind(n.role_locked)
    .bind(&n.village_code)
    .bind(&n.schedule_pattern)
    .bind(&n.action_profile)
    .bind(&n.created_by)
    .fetch_one(pool)
    .await?)
}

/// Write back every editable column of `row` (the caller validated it).
pub async fn save<'e, E>(exec: E, row: &NpcRow) -> Result<NpcRow>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    Ok(sqlx::query_as(&format!(
        "UPDATE npcs SET first_name = $2, last_name = $3, display_name = $4, enabled = $5,
             role = $6, role_locked = $7, village_code = $8, storekeeper_slot = $9,
             schedule_pattern = $10, action_profile = $11, place_where_work = $12, home = $13,
             meal_place = $14, mandatory_place = $15, action_timing = $16, action_config = $17,
             notes = $18, updated_at = now()
         WHERE id = $1 RETURNING {COLS}"
    ))
    .bind(row.id)
    .bind(&row.first_name)
    .bind(&row.last_name)
    .bind(&row.display_name)
    .bind(row.enabled)
    .bind(&row.role)
    .bind(row.role_locked)
    .bind(&row.village_code)
    .bind(row.storekeeper_slot)
    .bind(&row.schedule_pattern)
    .bind(&row.action_profile)
    .bind(&row.place_where_work)
    .bind(&row.home)
    .bind(&row.meal_place)
    .bind(&row.mandatory_place)
    .bind(&row.action_timing)
    .bind(&row.action_config)
    .bind(&row.notes)
    .fetch_one(exec)
    .await?)
}

/// Set `enabled` on several NPCs; returns the ids actually changed.
pub async fn set_enabled(pool: &PgPool, ids: &[Uuid], enabled: bool) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "UPDATE npcs SET enabled = $2, updated_at = now()
         WHERE id = ANY($1) AND enabled <> $2 RETURNING id",
    )
    .bind(ids)
    .bind(enabled)
    .fetch_all(pool)
    .await?)
}

/// Pick `count` NPCs to enable (oldest disabled first) or to disable (most
/// recent enabled non-storekeepers first, then storekeepers), within an
/// optional village / role filter.
pub async fn pick_for_toggle(
    pool: &PgPool,
    enable: bool,
    count: i64,
    village: Option<&str>,
    role: Option<&str>,
) -> Result<Vec<Uuid>> {
    let order = if enable {
        "created_at, id"
    } else {
        "(role = 'storekeeper'), created_at DESC, id DESC"
    };
    Ok(sqlx::query_scalar(&format!(
        "SELECT id FROM npcs
         WHERE enabled = $1
           AND ($2::TEXT IS NULL OR village_code = $2) AND ($3::TEXT IS NULL OR role = $3)
         ORDER BY {order} LIMIT $4"
    ))
    .bind(!enable)
    .bind(village)
    .bind(role)
    .bind(count.max(0))
    .fetch_all(pool)
    .await?)
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool> {
    let res = sqlx::query("DELETE FROM npcs WHERE id = $1").bind(id).execute(pool).await?;
    Ok(res.rows_affected() > 0)
}

pub async fn set_password(pool: &PgPool, id: Uuid, password_enc: &[u8]) -> Result<()> {
    sqlx::query(
        "UPDATE npcs SET password_enc = $2, password_key_id = $3, kc_state = 'ready', kc_error = NULL,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(password_enc)
    .bind(crate::crypto::KEY_ID)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a Keycloak login problem (`error`) or its resolution (`ready`).
/// Leaves `updated_at` alone: this is not a configuration change.
pub async fn set_kc_state(pool: &PgPool, id: Uuid, state: &str, error: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE npcs SET kc_state = $2, kc_error = $3 WHERE id = $1")
        .bind(id)
        .bind(state)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

/// Stamp a Horizon session start (and the very first one).
pub async fn touch_seen(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE npcs SET last_seen_at = now(), first_spawn_at = COALESCE(first_spawn_at, now())
         WHERE id = $1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Storekeeper slots taken in `village`, inside the caller's transaction.
pub async fn storekeeper_slots(tx: &mut Transaction<'_, Postgres>, village: &str) -> Result<Vec<i16>> {
    Ok(sqlx::query_scalar(
        "SELECT storekeeper_slot FROM npcs WHERE village_code = $1 AND storekeeper_slot IS NOT NULL
         ORDER BY storekeeper_slot",
    )
    .bind(village)
    .fetch_all(&mut **tx)
    .await?)
}

/// Serialise the storekeeper election of one village across concurrent spawns.
pub async fn lock_village(tx: &mut Transaction<'_, Postgres>, village: &str) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('npc-village:' || $1))")
        .bind(village)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Storekeeper count per village.
pub async fn storekeepers_by_village(pool: &PgPool) -> Result<Vec<(String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT village_code, count(*) FROM npcs
         WHERE village_code IS NOT NULL AND role = 'storekeeper' GROUP BY village_code",
    )
    .fetch_all(pool)
    .await?)
}

/// NPC count per village.
pub async fn count_by_village(pool: &PgPool) -> Result<Vec<(String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT village_code, count(*) FROM npcs WHERE village_code IS NOT NULL GROUP BY village_code",
    )
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn row() -> NpcRow {
        NpcRow {
            id: Uuid::new_v4(),
            login: "npc-abc123".into(),
            password_enc: vec![],
            password_key_id: 1,
            first_name: "Jean".into(),
            last_name: "Caisse".into(),
            display_name: "Jean Caisse".into(),
            enabled: true,
            role: "unassigned".into(),
            role_locked: false,
            village_code: None,
            storekeeper_slot: None,
            schedule_pattern: None,
            action_profile: vec![],
            place_where_work: None,
            home: None,
            meal_place: None,
            mandatory_place: None,
            action_timing: Json(json!({})),
            action_config: Json(json!({})),
            kc_state: "ready".into(),
            kc_error: None,
            first_spawn_at: None,
            last_seen_at: None,
            notes: None,
            created_by: None,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn empty_row_gives_a_bare_spec() {
        let r = row();
        let spec = r.to_spec().unwrap();
        assert_eq!(spec.uuid, r.id.to_string());
        assert_eq!(spec.name.as_deref(), Some("Jean Caisse"));
        assert!(spec.place_where_work.is_none());
        assert!(spec.schedule.is_none());
        assert!(r.assignment_pending());
    }

    #[test]
    fn full_row_maps_every_field() {
        let mut r = row();
        r.schedule_pattern = Some("magasinier_2".into());
        r.action_profile = vec!["going_to_work".into(), "depot_store_item".into()];
        r.place_where_work = Some(Json(json!({
            "uuid": "a9340169-767c-4b02-a2f3-67f2786375b3", "planet": "tarsis_3",
            "position": {"x": 1.85, "y": 0.479, "z": 6.913}
        })));
        r.action_timing = Json(json!({"take_a_walk": {"cooldown_ms": 1000}}));
        r.action_config = Json(json!({
            "depot_store_item": {"stand_distance_m": 1.5},
            "drive_to_factory": {"destination": "F0003"}
        }));
        let spec = r.to_spec().unwrap();
        assert_eq!(spec.schedule.as_deref(), Some("magasinier_2"));
        assert_eq!(spec.action_profile.len(), 2);
        assert_eq!(spec.place_where_work.unwrap().position.z, 6.913);
        assert_eq!(spec.action_timing["take_a_walk"].cooldown_ms, Some(1000));
        assert_eq!(spec.depot_store_item.unwrap().stand_distance_m, Some(1.5));
        assert_eq!(spec.drive_to_factory.unwrap().destination.as_deref(), Some("F0003"));
    }

    #[test]
    fn rejects_unknown_config_and_bad_places() {
        let mut r = row();
        r.action_config = Json(json!({"fly": {}}));
        assert!(r.to_spec().is_err());

        let mut r = row();
        r.place_where_work = Some(Json(json!({"uuid": "x"})));
        assert!(r.to_spec().is_err(), "position is required");
    }

    #[test]
    fn locked_or_assigned_rows_skip_assignment() {
        let mut r = row();
        r.role_locked = true;
        assert!(!r.assignment_pending());
        let mut r = row();
        r.role = "villager".into();
        assert!(!r.assignment_pending());
    }
}
