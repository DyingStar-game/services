//! STUB action. Registered for completeness; `can_run` returns false so it
//! never fires. Replace with a real driving loop later.

use async_trait::async_trait;

use super::{Action, ActionContext};
use crate::error::Result;

pub struct DriveTruck;

#[async_trait]
impl Action for DriveTruck {
    fn name(&self) -> &'static str {
        "drive_truck"
    }

    fn can_run(&self, _ctx: &ActionContext) -> bool {
        false
    }

    async fn run(&self, _ctx: &mut ActionContext) -> Result<()> {
        // TODO: drive a truck along a configured path.
        Ok(())
    }
}
