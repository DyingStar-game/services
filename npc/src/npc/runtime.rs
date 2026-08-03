//! NPC runtime: one tokio task per NPC. Connects to Horizon (with reconnect),
//! maintains a [`WorldView`] from inbound GORC frames, and runs an
//! [`ActionRegistry`] scheduler on a fixed tick.

use std::{sync::Arc, time::Duration};

use rand::{Rng, SeedableRng, rngs::StdRng};
use tokio::{
    sync::{Mutex, oneshot},
    time,
};
use tracing::{debug, info, warn};

use crate::{
    actions::{ActionContext, ActionRegistry},
    config::Config,
    horizon::{
        client::{HorizonClient, HorizonReceiver, HorizonSender},
        messages::StandardizedFrame,
    },
    jwt,
    npc::{
        spec::NpcSpec,
        state::{NpcState, NpcStatus},
    },
    world::view::WorldView,
};

pub struct NpcHandle {
    pub name: String,
    pub spec: NpcSpec,
    pub stop_tx: oneshot::Sender<()>,
    pub join: tokio::task::JoinHandle<()>,
}

pub async fn run(
    config: Arc<Config>,
    name: String,
    spec: NpcSpec,
    mut stop_rx: oneshot::Receiver<()>,
) {
    info!(npc = %name, actions = ?spec.action_profile, "NPC runtime started");

    // Random stagger delay (1 000 – 60 000 ms) so NPCs don't all connect at once.
    let delay_ms = rand::thread_rng().gen_range(1_000u64..=60_000u64);
    info!(npc = %name, delay_ms, "waiting before initial connect");
    tokio::select! {
        biased;
        _ = &mut stop_rx => {
            info!(npc = %name, "stop requested during initial delay");
            return;
        }
        _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
    }

    let tick = Duration::from_millis(config.tick_interval_ms);
    let startup_delay = Duration::from_millis(config.action_startup_delay_ms);

    loop {
        // Generate a fresh JWT before each connection attempt.
        let display_name = spec.name.as_deref().unwrap_or(&name);
        let token = match jwt::generate(&spec.uuid, display_name, &config.jwt_secret) {
            Ok(t) => t,
            Err(e) => {
                warn!(npc = %name, error = %e, "cannot generate JWT; retrying");
                tokio::time::sleep(Duration::from_millis(config.reconnect_backoff_initial_ms)).await;
                continue;
            }
        };

        // Connect (with backoff) until we have a live session, or stop.
        let session = tokio::select! {
            biased;
            _ = &mut stop_rx => {
                info!(npc = %name, "stop requested before connection");
                return;
            }
            t = HorizonClient::connect_with_retry(&config, &config.horizon_url, &token) => t,
        };
        let (sender, mut receiver, my_uuid) = session;
        let world = Arc::new(WorldView::new(my_uuid.clone()));
        // Shared per-NPC state (FSM status + blackboard), built fresh per
        // session alongside the WorldView: a reconnect respawns the NPC, so
        // its lifecycle starts clean. An NPC that works spawns in its
        // habitation; `going_to_work` walks it out from there.
        let state = Arc::new(NpcState::new());
        if spec.place_where_work.is_some() {
            state.set_status(NpcStatus::AtHome);
        }
        info!(
            npc = %name,
            %my_uuid,
            settle_ms = config.action_startup_delay_ms,
            "Horizon session established; settling before running actions"
        );

        // Actions are rebuilt per session, alongside the WorldView: a fresh
        // connection means a freshly spawned NPC, so per-session action state
        // (e.g. "already went to work") starts clean.
        let registry = Arc::new(ActionRegistry::from_spec(&spec));

        let current = Arc::new(Mutex::new(None::<tokio::task::JoinHandle<()>>));

        let outcome = session_loop(
            &name,
            tick,
            startup_delay,
            Arc::clone(&world),
            Arc::clone(&state),
            sender.clone(),
            &mut receiver,
            Arc::clone(&registry),
            Arc::clone(&current),
            &mut stop_rx,
        )
        .await;

        if let Some(h) = current.lock().await.take() {
            h.abort();
        }

        match outcome {
            SessionOutcome::Stop => {
                info!(npc = %name, "stop requested; exiting runtime");
                return;
            }
            SessionOutcome::Disconnected => {
                warn!(npc = %name, "disconnected from Horizon; reconnecting");
                continue;
            }
        }
    }
}

enum SessionOutcome {
    Stop,
    Disconnected,
}

#[allow(clippy::too_many_arguments)]
async fn session_loop(
    name: &str,
    tick: Duration,
    startup_delay: Duration,
    world: Arc<WorldView>,
    state: Arc<NpcState>,
    sender: HorizonSender,
    receiver: &mut HorizonReceiver,
    registry: Arc<ActionRegistry>,
    current: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    stop_rx: &mut oneshot::Receiver<()>,
) -> SessionOutcome {
    // Hold off on the first tick so the NPC settles in before acting. Only the
    // scheduler waits — the loop below keeps applying inbound frames, so the
    // world is already populated when the first action runs.
    let mut ticker = time::interval_at(time::Instant::now() + startup_delay, tick);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = &mut *stop_rx => return SessionOutcome::Stop,

            maybe_frame = receiver.recv() => {
                match maybe_frame {
                    Some(StandardizedFrame::Gorc(g)) => {
                        let _ = world.apply(&g);
                    }
                    Some(StandardizedFrame::InitAck { player_id }) => {
                        debug!(npc = %name, %player_id, "stray init_ack");
                    }
                    Some(StandardizedFrame::Error { message }) => {
                        warn!(npc = %name, %message, "horizon error");
                        return SessionOutcome::Disconnected;
                    }
                    Some(StandardizedFrame::Other { name: ev, .. }) => {
                        debug!(npc = %name, event = %ev, "ignoring non-GORC frame");
                    }
                    None => return SessionOutcome::Disconnected,
                }
            }

            _ = ticker.tick() => {
                let mut slot = current.lock().await;
                if let Some(h) = slot.as_ref() {
                    if h.is_finished() {
                        slot.take();
                    }
                }
                if slot.is_some() {
                    continue;
                }

                let probe = ActionContext {
                    npc_name: name.to_string(),
                    world: Arc::clone(&world),
                    state: Arc::clone(&state),
                    send: sender.clone(),
                    rng: StdRng::from_entropy(),
                };
                let Some(action) = registry.pick(&probe) else {
                    continue;
                };

                let action_name = action.name();
                debug!(npc = %name, action = %action_name, "starting action");

                // Stamp the start before spawning so the `once`/`min_interval`
                // gates see this run on the very next tick.
                registry.mark_started(action_name);
                let run_timeout = registry.timing_of(action_name).and_then(|t| t.timeout);

                let name_owned = name.to_string();
                let action_name_owned = action_name.to_string();
                let registry_for_task = Arc::clone(&registry);
                let mut ctx = ActionContext {
                    npc_name: name.to_string(),
                    world: Arc::clone(&world),
                    state: Arc::clone(&state),
                    send: sender.clone(),
                    rng: StdRng::from_entropy(),
                };
                let action_clone = Arc::clone(&action);
                let h = tokio::spawn(async move {
                    let result = match run_timeout {
                        Some(d) => match time::timeout(d, action_clone.run(&mut ctx)).await {
                            Ok(r) => r,
                            Err(_) => {
                                warn!(
                                    npc = %name_owned,
                                    action = %action_name_owned,
                                    timeout_ms = d.as_millis(),
                                    "action timed out; aborting run"
                                );
                                Ok(())
                            }
                        },
                        None => action_clone.run(&mut ctx).await,
                    };
                    if let Err(e) = result {
                        warn!(npc = %name_owned, action = %action_name_owned, error = %e, "action failed");
                    }
                    // Stamp completion so the cooldown gate measures from here.
                    registry_for_task.mark_finished(&action_name_owned);
                });
                *slot = Some(h);
            }
        }
    }
}
