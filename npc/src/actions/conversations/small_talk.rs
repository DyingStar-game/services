//! STUB conversation — `can_run`/`run` are not wired up yet.

use async_trait::async_trait;

use super::Conversation;
use crate::{actions::ActionContext, error::Result};

pub struct SmallTalk;

#[async_trait]
impl Conversation for SmallTalk {
    fn name(&self) -> &'static str {
        "small_talk"
    }

    async fn run(
        &self,
        _ctx: &mut ActionContext,
        _target_uuid: &str,
        _target_name: Option<&str>,
    ) -> Result<()> {
        // TODO: implement multi-turn small talk.
        Ok(())
    }
}
