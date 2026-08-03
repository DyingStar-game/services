use std::sync::Arc;

use rand::rngs::StdRng;

use crate::{horizon::client::HorizonSender, npc::state::NpcState, world::view::WorldView};

/// Per-tick context handed to every [`Action`](super::Action). Carries the
/// NPC's view of the world, its shared mutable state, plus a sender to talk
/// back to Horizon.
pub struct ActionContext {
    pub npc_name: String,
    pub world: Arc<WorldView>,
    pub state: Arc<NpcState>,
    pub send: HorizonSender,
    pub rng: StdRng,
}
