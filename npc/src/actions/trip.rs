use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long to wait for the server's `npc_arrived` notice before walking the
/// NPC to its destination again. Deliberately generous — a trip across the
/// settlement takes about a minute — because a needless re-dispatch restarts
/// the walk from wherever the NPC had got to.
pub const ARRIVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// Watchdog over a dispatched walk, so a trip the server never completes is
/// retried instead of stranding the NPC where it stands.
///
/// `npc_go_to_position` is fire-and-forget: the NPC leaves its travelling
/// status only when `npc_arrived` replicates back. When that never happens —
/// the order was dropped, or the server could not path out of wherever the NPC
/// spawned — the travel actions' `can_run` waits on a flag that will not come,
/// and the NPC stands still for the rest of its shift without a single log
/// line. This turns that silence into a retry and a warning.
#[derive(Default)]
pub struct TripWatch {
    dispatched_at: Mutex<Option<Instant>>,
}

impl TripWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stamp a freshly dispatched walk.
    pub fn dispatched(&self) {
        *self.dispatched_at.lock().unwrap() = Some(Instant::now());
    }

    /// Forget the walk in flight: it arrived, so nothing is owed.
    pub fn arrived(&self) {
        *self.dispatched_at.lock().unwrap() = None;
    }

    /// Whether the walk in flight has gone unanswered for [`ARRIVAL_TIMEOUT`].
    /// `false` when no walk is in flight.
    pub fn timed_out(&self) -> bool {
        self.dispatched_at
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() >= ARRIVAL_TIMEOUT)
    }

    /// How long the walk in flight has been unanswered, for logging.
    pub fn waiting_s(&self) -> u64 {
        self.dispatched_at
            .lock()
            .unwrap()
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    /// Backdate the stamp so a test can reach the timeout without sleeping
    /// (the stamps are `std::time::Instant`, which no test clock moves).
    #[cfg(test)]
    pub fn age_by(&self, age: Duration) {
        let mut slot = self.dispatched_at.lock().unwrap();
        if let Some(t) = *slot {
            *slot = Some(t - age);
        }
    }
}
