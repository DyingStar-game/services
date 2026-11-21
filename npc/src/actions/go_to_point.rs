use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::time;
use tracing::debug;

use super::{Action, ActionContext};
use crate::{error::Result, horizon::messages::Vec3};

/// Roams toward a fixed target point by emitting `movement/update_velocity`
/// frames at ~10 Hz until close enough, then stops. Only eligible when no
/// players are around (per the "no actions when alone" rule, plus this is
/// roaming behaviour the NPC does *while* alone).
pub struct GoToPoint {
    pub target: Vec3,
    pub epsilon: f64,
    pub tick: Duration,
}

impl Default for GoToPoint {
    fn default() -> Self {
        Self {
            target: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            epsilon: 0.5,
            tick: Duration::from_millis(100),
        }
    }
}

#[async_trait]
impl Action for GoToPoint {
    fn name(&self) -> &'static str {
        "go_to_point"
    }

    fn priority(&self) -> i32 {
        10
    }

    fn can_run(&self, ctx: &ActionContext) -> bool {
        // Roam only when nobody is watching.
        !ctx.world.has_other_player()
    }

    async fn run(&self, ctx: &mut ActionContext) -> Result<()> {
        loop {
            // Bail out the moment a player enters the zone.
            if ctx.world.has_other_player() {
                send_stop(ctx).await?;
                return Ok(());
            }

            // Use the NPC's own position if Horizon echoes it back into the
            // WorldView; otherwise fall back to "always moving toward target"
            // by sending the direction unit vector for one tick and exiting.
            let here = current_position(ctx);
            let dx = self.target.x - here.x;
            let dz = self.target.z - here.z;
            let dist = (dx * dx + dz * dz).sqrt();
            if dist <= self.epsilon {
                send_stop(ctx).await?;
                return Ok(());
            }

            let ux = dx / dist;
            let uz = dz / dist;
            ctx.send
                .send(
                    "movement",
                    "update_velocity",
                    json!({
                        "pos": { "x": ux, "y": uz },
                        "rot": { "x": 0.0, "y": 0.0, "z": 0.0 },
                        "uuid": ctx.world.my_uuid,
                    }),
                )
                .await?;

            time::sleep(self.tick).await;
        }
    }
}

fn current_position(ctx: &ActionContext) -> Vec3 {
    ctx.world
        .objects
        .get(&ctx.world.my_uuid)
        .map(|o| o.position)
        .unwrap_or_default()
}

async fn send_stop(ctx: &ActionContext) -> Result<()> {
    debug!(npc = %ctx.npc_name, "go_to_point: stopping");
    ctx.send
        .send(
            "movement",
            "update_velocity",
            json!({
                "pos": { "x": 0.0, "y": 0.0 },
                "rot": { "x": 0.0, "y": 0.0, "z": 0.0 },
                "uuid": ctx.world.my_uuid,
            }),
        )
        .await
}
