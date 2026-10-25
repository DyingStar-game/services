use std::time::Duration;

/// Declarative "life" of an [`Action`](super::Action): how often it may start
/// and how long a single run may last. The scheduler enforces every field, so
/// an action describes its cadence here instead of hand-rolling flags and
/// cooldowns in its own struct.
///
/// `Default` (all `None`/`false`) means "no restriction": eligible on every
/// tick its `can_run` allows, no timeout. Existing actions keep that behaviour
/// until they opt in.
#[derive(Debug, Clone, Default)]
pub struct ActionTiming {
    /// Run at most once per Horizon session. Replaces per-action "already ran"
    /// flags such as `GoingToWork`'s `AtomicBool`.
    pub once: bool,

    /// Minimum gap between the *end* of one run and the *start* of the next.
    /// Note this is per-action, not per-target: an action that must throttle
    /// per player (e.g. greeting) still tracks that itself.
    pub cooldown: Option<Duration>,

    /// Minimum gap between the *start* of one run and the *start* of the next,
    /// i.e. a periodic cadence regardless of how long each run takes.
    pub min_interval: Option<Duration>,

    /// Hard cap on a single run. When set, the scheduler aborts the run if it
    /// exceeds this, freeing the NPC's single action slot.
    pub timeout: Option<Duration>,
}
