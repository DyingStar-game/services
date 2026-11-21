use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use super::Conversation;
use crate::{actions::ActionContext, error::Result};

/// Sends one short "hello" via `player/client_action`. The Godot server is
/// expected to route this to the target player as a chat message.
pub struct Greet;

#[async_trait]
impl Conversation for Greet {
    fn name(&self) -> &'static str {
        "greet"
    }

    async fn run(
        &self,
        ctx: &mut ActionContext,
        target_uuid: &str,
        target_name: Option<&str>,
    ) -> Result<()> {
        // Fall back to a nameless hello rather than blurting out a UUID.
        let text = match target_name {
            Some(name) => format!("Hello {name}"),
            None => "Hello".to_string(),
        };
        info!(npc = %ctx.npc_name, target = %target_uuid, %text, "greet");
        ctx.send
            .send(
                "player",
                "client_action",
                json!({
                    "action": "dialog",
                    "to": target_uuid,
                    "text": text,
                    "uuid": ctx.world.my_uuid,
                }),
            )
            .await?;
        Ok(())
    }
}
