//! Keycloak Admin REST calls on the NPC users, as `svc-npc`. Its fine-grained
//! permissions only cover the members of group `npcs`, which carries the realm
//! role `npc`: a user created here is put in that group and gets the role
//! through it.

use std::sync::Arc;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::ServiceToken;
use crate::error::{NpcError, Result};

/// Group every NPC user belongs to.
pub const NPC_GROUP: &str = "npcs";

pub struct KcAdmin {
    http: reqwest::Client,
    base: String,
    token: Arc<ServiceToken>,
}

/// The profile of a new NPC user.
pub struct NewUser<'a> {
    pub login: &'a str,
    pub first_name: &'a str,
    pub last_name: &'a str,
    pub display_name: &'a str,
    pub password: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct GroupMember {
    pub id: Uuid,
    pub username: String,
}

impl KcAdmin {
    pub fn new(http: reqwest::Client, keycloak_url: &str, realm: &str, token: Arc<ServiceToken>) -> Self {
        Self { http, base: format!("{keycloak_url}/admin/realms/{realm}"), token }
    }

    async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<reqwest::Response> {
        for attempt in 0..2 {
            let bearer = self.token.bearer().await?;
            let mut req = self.http.request(method.clone(), format!("{}{path}", self.base)).bearer_auth(bearer);
            if let Some(b) = body {
                req = req.json(b);
            }
            let res = req.send().await.map_err(|e| NpcError::upstream("keycloak", 0, e.to_string()))?;
            if res.status() == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.token.invalidate().await;
                continue;
            }
            return Ok(res);
        }
        unreachable!()
    }

    async fn fail(res: reqwest::Response, what: &str) -> NpcError {
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        match status {
            409 => NpcError::Conflict(format!("{what}: {body}")),
            403 => NpcError::upstream(
                "keycloak",
                403,
                format!("{what}: svc-npc is not allowed (fine-grained permissions on group '{NPC_GROUP}'?) {body}"),
            ),
            _ => NpcError::upstream("keycloak", status, format!("{what}: {body}")),
        }
    }

    /// Create an NPC user in group `npcs`; returns its id (the future `sub`).
    pub async fn create_user(&self, u: &NewUser<'_>) -> Result<Uuid> {
        let body = user_payload(u);
        let res = self.call(Method::POST, "/users", Some(&body)).await?;
        if res.status() != StatusCode::CREATED {
            return Err(Self::fail(res, "create user").await);
        }
        let location = res
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        location
            .rsplit('/')
            .next()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| NpcError::upstream("keycloak", 201, format!("no user id in Location '{location}'")))
    }

    pub async fn reset_password(&self, id: Uuid, password: &str) -> Result<()> {
        let body = json!({ "type": "password", "value": password, "temporary": false });
        let res = self.call(Method::PUT, &format!("/users/{id}/reset-password"), Some(&body)).await?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "reset password").await);
        }
        Ok(())
    }

    /// Rename (first/last/display name). Keycloak treats a PUT as the whole
    /// profile — a field left out (the email) is erased and the account can no
    /// longer log in ("Account is not fully set up") — so the current
    /// representation is read, edited and sent back whole.
    pub async fn update_profile(&self, id: Uuid, first: &str, last: &str, display: &str) -> Result<()> {
        let res = self.call(Method::GET, &format!("/users/{id}"), None).await?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "read user").await);
        }
        let mut user: Value = res.json().await.map_err(|e| NpcError::upstream("keycloak", 0, e.to_string()))?;
        rename(&mut user, first, last, display);
        let res = self.call(Method::PUT, &format!("/users/{id}"), Some(&user)).await?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "update user").await);
        }
        Ok(())
    }

    /// Delete an NPC user. Already gone counts as done.
    pub async fn delete_user(&self, id: Uuid) -> Result<()> {
        let res = self.call(Method::DELETE, &format!("/users/{id}"), None).await?;
        if res.status().is_success() || res.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(Self::fail(res, "delete user").await)
    }

    /// End every session of an NPC user (best effort, on despawn).
    pub async fn logout(&self, id: Uuid) -> Result<()> {
        let res = self.call(Method::POST, &format!("/users/{id}/logout"), None).await?;
        if res.status().is_success() || res.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(Self::fail(res, "logout user").await)
    }

    /// Members of group `npcs`, to spot accounts without a database row.
    pub async fn list_group_members(&self) -> Result<Vec<GroupMember>> {
        // By path: a group search needs `query-groups`, which svc-npc lacks.
        let res = self.call(Method::GET, &format!("/group-by-path/{NPC_GROUP}"), None).await?;
        if !res.status().is_success() {
            return Err(Self::fail(res, "find group").await);
        }
        let group: Value = res.json().await.map_err(|e| NpcError::upstream("keycloak", 0, e.to_string()))?;
        let Some(gid) = group["id"].as_str().map(str::to_string) else {
            return Err(NpcError::upstream("keycloak", 404, format!("group '{NPC_GROUP}' not found")));
        };
        let mut members = Vec::new();
        let page = 200;
        loop {
            let res = self
                .call(
                    Method::GET,
                    &format!("/groups/{gid}/members?first={}&max={page}&briefRepresentation=true", members.len()),
                    None,
                )
                .await?;
            if !res.status().is_success() {
                return Err(Self::fail(res, "list group members").await);
            }
            let batch: Vec<GroupMember> =
                res.json().await.map_err(|e| NpcError::upstream("keycloak", 0, e.to_string()))?;
            let n = batch.len();
            members.extend(batch);
            if n < page {
                return Ok(members);
            }
        }
    }
}

/// Set the names on a full user representation, keeping everything else.
fn rename(user: &mut Value, first: &str, last: &str, display: &str) {
    user["firstName"] = json!(first);
    user["lastName"] = json!(last);
    if !user["attributes"].is_object() {
        user["attributes"] = json!({});
    }
    user["attributes"]["display_name"] = json!([display]);
    // Read-only parts of the representation the PUT does not take.
    if let Some(u) = user.as_object_mut() {
        u.remove("access");
    }
}

/// Body of `POST /users`. Keycloak refuses to log in an account missing its
/// email or names ("Account is not fully set up"), hence the synthetic email.
fn user_payload(u: &NewUser<'_>) -> Value {
    json!({
        "username": u.login,
        "email": format!("{}@npc.dyingstar.invalid", u.login),
        "emailVerified": true,
        "firstName": u.first_name,
        "lastName": u.last_name,
        "enabled": true,
        "attributes": { "display_name": [u.display_name] },
        "groups": [format!("/{NPC_GROUP}")],
        "credentials": [{ "type": "password", "value": u.password, "temporary": false }],
        "requiredActions": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_keeps_the_rest_of_the_profile() {
        let mut u = json!({
            "id": "x", "username": "npc-1", "email": "npc-1@npc.dyingstar.invalid", "emailVerified": true,
            "firstName": "Jean", "lastName": "Caisse", "attributes": {"display_name": ["Jean Caisse"], "other": ["k"]},
            "access": {"manage": true},
        });
        rename(&mut u, "Alain", "Deloin", "Alain Deloin");
        assert_eq!(u["email"], "npc-1@npc.dyingstar.invalid");
        assert_eq!(u["emailVerified"], true);
        assert_eq!(u["firstName"], "Alain");
        assert_eq!(u["attributes"]["display_name"][0], "Alain Deloin");
        assert_eq!(u["attributes"]["other"][0], "k");
        assert!(u.get("access").is_none());
    }

    #[test]
    fn user_payload_is_complete() {
        let p = user_payload(&NewUser {
            login: "npc-3f2a9c",
            first_name: "Jean",
            last_name: "Caisse",
            display_name: "Jean Caisse",
            password: "pw",
        });
        assert_eq!(p["username"], "npc-3f2a9c");
        assert_eq!(p["email"], "npc-3f2a9c@npc.dyingstar.invalid");
        assert_eq!(p["emailVerified"], true);
        assert_eq!(p["firstName"], "Jean");
        assert_eq!(p["attributes"]["display_name"][0], "Jean Caisse");
        assert_eq!(p["groups"][0], "/npcs");
        assert_eq!(p["credentials"][0]["temporary"], false);
        assert_eq!(p["requiredActions"], json!([]));
    }
}
