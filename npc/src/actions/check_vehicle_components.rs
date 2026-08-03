//! STUB action.

use async_trait::async_trait;

use super::{Action, ActionContext};
use crate::error::Result;

pub struct CheckVehicleComponents;

#[async_trait]
impl Action for CheckVehicleComponents {
    fn name(&self) -> &'static str {
        "check_vehicle_components"
    }

    fn can_run(&self, _ctx: &ActionContext) -> bool {
        false
    }

    async fn run(&self, _ctx: &mut ActionContext) -> Result<()> {
        // TODO: walk around a vehicle, inspect each component.
        Ok(())
    }
}
