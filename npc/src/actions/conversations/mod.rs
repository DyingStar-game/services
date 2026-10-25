pub mod greet;
pub mod small_talk;

use async_trait::async_trait;

use crate::{actions::ActionContext, error::Result};

/// A conversation an NPC can have with one nearby player.
#[async_trait]
pub trait Conversation: Send + Sync {
    fn name(&self) -> &'static str;
    /// `target_name` is the player's display name, or `None` when Horizon has
    /// not replicated one yet — conversations must still work without it.
    async fn run(
        &self,
        ctx: &mut ActionContext,
        target_uuid: &str,
        target_name: Option<&str>,
    ) -> Result<()>;
}
