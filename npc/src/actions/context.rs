use std::{collections::HashSet, sync::Arc};

use rand::rngs::StdRng;

use crate::{
    depot::DepotRole, horizon::client::HorizonSender, npc::state::NpcState,
    world::view::WorldView,
};

/// Per-tick context handed to every [`Action`](super::Action). Carries the
/// NPC's view of the world, its shared mutable state, plus a sender to talk
/// back to Horizon.
pub struct ActionContext {
    pub npc_name: String,
    pub world: Arc<WorldView>,
    pub state: Arc<NpcState>,
    pub send: HorizonSender,
    pub rng: StdRng,
    /// Action names the shift schedule permits at the current planetary hour.
    /// `None` = no schedule gating (the NPC has no schedule, or the pattern is
    /// unknown), so every action in the profile is eligible.
    pub allowed_actions: Option<HashSet<String>>,
    /// This NPC's cargo-depot role this tick (reception vs stocker), derived from
    /// the depot's `reception_owner` claim or the deterministic fallback. Read by
    /// the two depot actions to split the work between magasiniers.
    pub depot_role: DepotRole,
}
