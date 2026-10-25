//! Creating NPCs: a generated name, a Keycloak account in group `npcs` with a
//! random password, then the database row (keyed by the Keycloak user id, the
//! future JWT `sub`). A failed insert deletes the account again.

use rand::thread_rng;
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

use crate::{
    crypto::random_password,
    db::{self, npcs::{NewNpc, NpcRow}},
    error::{NpcError, Result},
    keycloak::admin::NewUser,
    npc::{manager::NpcManager, names::generate_login},
};

pub const MAX_BATCH: u32 = 50;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    /// How many NPCs (1..=50).
    #[serde(default = "one")]
    pub count: u32,
    /// `unassigned` (default: storekeeper or villager decided at first spawn),
    /// `villager` or `custom` (no automatic assignment, configured by hand).
    #[serde(default)]
    pub role: Option<String>,
    /// Spawn them right away (when the global switch is on).
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub schedule_pattern: Option<String>,
    /// For `custom`: the actions it may run.
    #[serde(default)]
    pub action_profile: Option<Vec<String>>,
    /// Imposed names (first, last) instead of generated ones; `count` must be 1.
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
}

fn one() -> u32 {
    1
}

impl CreateRequest {
    pub fn validate(&self) -> Result<()> {
        if self.count == 0 || self.count > MAX_BATCH {
            return Err(NpcError::BadRequest(format!("count must be within 1..={MAX_BATCH}")));
        }
        if let Some(r) = &self.role {
            if !["unassigned", "villager", "custom"].contains(&r.as_str()) {
                return Err(NpcError::BadRequest("role must be unassigned, villager or custom".into()));
            }
        }
        if (self.first_name.is_some() || self.last_name.is_some()) && self.count != 1 {
            return Err(NpcError::BadRequest("imposed names need count = 1".into()));
        }
        Ok(())
    }
}

/// Create one NPC. Names clash rarely; a clash draws again (up to 10 times).
pub async fn create_one(mgr: &NpcManager, req: &CreateRequest, by: Option<&str>) -> Result<NpcRow> {
    let (first, last) = pick_name(mgr, req).await?;
    let display = format!("{first} {last}");
    let password = random_password();

    // A login clash in Keycloak (409) draws another one.
    let mut created = None;
    for _ in 0..3 {
        let login = generate_login(&mut thread_rng());
        let user = NewUser {
            login: &login,
            first_name: &first,
            last_name: &last,
            display_name: &display,
            password: &password,
        };
        match mgr.kc.create_user(&user).await {
            Ok(id) => {
                created = Some((id, login));
                break;
            }
            Err(NpcError::Conflict(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    let (id, login) = created.ok_or_else(|| NpcError::Conflict("no free login after 3 tries".into()))?;

    let role = req.role.clone().unwrap_or_else(|| "unassigned".into());
    let row = async {
        let sealed = mgr.passwords.seal(id, &password)?;
        let new = NewNpc {
            id,
            login: login.clone(),
            password_enc: sealed,
            first_name: first.clone(),
            last_name: last.clone(),
            display_name: display.clone(),
            enabled: req.enabled,
            role: role.clone(),
            // A role chosen by hand is not overridden at first spawn.
            role_locked: role != "unassigned",
            village_code: None,
            schedule_pattern: req.schedule_pattern.clone(),
            action_profile: req.action_profile.clone().unwrap_or_default(),
            created_by: by.map(str::to_string),
        };
        let mut row = db::npcs::insert(mgr.pool(), &new).await?;
        if role == "villager" {
            let schedule = req.schedule_pattern.clone().or_else(|| mgr.roles.villager.schedule.clone());
            mgr.roles.apply_villager(&mut row, None, schedule);
            row = db::npcs::save(mgr.pool(), &row).await?;
        }
        Ok::<_, NpcError>(row)
    }
    .await;

    match row {
        Ok(row) => {
            info!(login = %row.login, name = %row.display_name, "NPC created");
            mgr.events().info(
                row.id,
                "created",
                format!("{} created ({})", row.display_name, row.login),
                Some(json!({ "by": by, "role": row.role })),
            );
            Ok(row)
        }
        Err(e) => {
            warn!(%login, error = %e, "NPC row insert failed; deleting its Keycloak account");
            if let Err(e2) = mgr.kc.delete_user(id).await {
                warn!(%login, error = %e2, "could not delete the orphan Keycloak account");
            }
            Err(e)
        }
    }
}

async fn pick_name(mgr: &NpcManager, req: &CreateRequest) -> Result<(String, String)> {
    if req.first_name.is_some() || req.last_name.is_some() {
        let first = req.first_name.clone().unwrap_or_default().trim().to_string();
        let last = req.last_name.clone().unwrap_or_default().trim().to_string();
        if first.is_empty() || last.is_empty() {
            return Err(NpcError::BadRequest("firstName and lastName are both required".into()));
        }
        if db::npcs::display_name_taken(mgr.pool(), &format!("{first} {last}")).await? {
            return Err(NpcError::Conflict(format!("an NPC is already named {first} {last}")));
        }
        return Ok((first, last));
    }
    for _ in 0..10 {
        let (first, last) = mgr.names.generate(&mut thread_rng());
        if !db::npcs::display_name_taken(mgr.pool(), &format!("{first} {last}")).await? {
            return Ok((first, last));
        }
    }
    Err(NpcError::Conflict("no free generated name after 10 tries (extend config/names)".into()))
}
