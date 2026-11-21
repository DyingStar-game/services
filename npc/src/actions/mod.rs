pub mod context;
pub mod registry;
pub mod timing;
pub mod trip;

pub mod depot_common;
pub mod driving;
pub mod depot_reception_coordinator;
pub mod depot_store_item;
pub mod depot_store_reception_to_temp_area;
pub mod detect_player_and_greet;
pub mod drive_to_factory;
pub mod go_to_place;
pub mod go_to_point;
pub mod going_to_work;
pub mod manual;
pub mod reception_players;
pub mod take_a_walk;

// stubs — compile, registered, but `can_run` returns false
pub mod check_vehicle_components;
pub mod go_sell_object;

pub mod conversations;

use async_trait::async_trait;

use crate::error::Result;
pub use context::ActionContext;
pub use registry::ActionRegistry;
pub use timing::ActionTiming;

/// Anything an NPC can decide to do. Implementations live in one file per
/// behavior under `src/actions/`. The scheduler polls `can_run` on every tick
/// and `await`s `run` for the chosen action.
#[async_trait]
pub trait Action: Send + Sync {
    /// Unique short identifier — matches entries in `NpcSpec.action_profile`.
    fn name(&self) -> &'static str;

    /// Higher priority wins when multiple actions are eligible. Default 0.
    fn priority(&self) -> i32 {
        0
    }

    /// Declarative cadence and run timeout. The scheduler enforces this on top
    /// of `can_run`; default is unrestricted. See [`ActionTiming`].
    fn timing(&self) -> ActionTiming {
        ActionTiming::default()
    }

    /// Cheap synchronous predicate: may this action start right now?
    fn can_run(&self, ctx: &ActionContext) -> bool;

    /// Execute the action. Should `await` Horizon I/O and may run for any
    /// duration. The scheduler will not start a new action until this returns.
    async fn run(&self, ctx: &mut ActionContext) -> Result<()>;
}
