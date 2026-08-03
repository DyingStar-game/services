//! STUB action.

use async_trait::async_trait;

use super::{Action, ActionContext};
use crate::error::Result;

pub struct GoSellObject;

#[async_trait]
impl Action for GoSellObject {
    fn name(&self) -> &'static str {
        "go_sell_object"
    }

    fn can_run(&self, _ctx: &ActionContext) -> bool {
        false
    }

    async fn run(&self, _ctx: &mut ActionContext) -> Result<()> {
        // TODO: travel to a vendor and offload an item from inventory.
        Ok(())
    }
}
