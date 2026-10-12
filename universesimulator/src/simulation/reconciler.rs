//! Learns mission outcomes by polling the mission service and moves the goods (ADR 0007).

use std::collections::HashMap;

use tracing::{info, warn};
use uuid::Uuid;

use super::TickReport;
use crate::clients::Holder;
use crate::db::{catalog::Federation, orders::{self, Leg}, world};
use crate::error::{AppError, Result};
use crate::state::AppState;

pub async fn run(st: &AppState, fed: &Federation, report: &mut TickReport) -> Result<()> {
    let published: HashMap<Uuid, Leg> = orders::legs_by_status(&st.pool, fed.id, &["published"])
        .await?
        .into_iter()
        .filter_map(|l| l.mission_id.map(|m| (m, l)))
        .collect();
    if published.is_empty() {
        return Ok(());
    }
    // A mission is created just before its leg is marked published: missions older than the
    // oldest published leg (with a margin) cannot be ours.
    let oldest = published.values().filter_map(|l| l.published_at).min().map(|t| t - time::Duration::hours(1));
    for status in ["completed", "cancelled", "expired"] {
        for m in st.clients.mission.list("politics", fed.political_entity_id, status, oldest).await? {
            let Some(leg) = published.get(&m.id) else { continue };
            let result = match status {
                "completed" => complete(st, leg).await.map(|_| report.legs_completed += 1),
                _ => retry_or_fail(st, leg, status, report).await,
            };
            if let Err(e) = result {
                orders::set_leg_error(&st.pool, leg.id, &e.to_string()).await?;
                report.error("reconcile leg", e);
            }
        }
    }
    Ok(())
}

async fn complete(st: &AppState, leg: &Leg) -> Result<()> {
    let order = orders::get_order(&st.pool, leg.order_id).await?.ok_or_else(|| AppError::NotFound("order".into()))?;
    if let (Some(hold), Some(to)) = (leg.hold_id, leg.to_warehouse_id) {
        match st.clients.inventory.consume_hold(hold, Holder::warehouse(to)).await {
            Ok(()) => {}
            // Consumed by a previous attempt whose bookkeeping failed.
            Err(e) if e.upstream_status() == Some(409) => warn!(leg = %leg.id, error = %e, "hold already consumed"),
            Err(e) => return Err(e),
        }
    }
    let mut tx = st.pool.begin().await?;
    if order.kind == "mining" {
        world::consume_mineral_source(&mut tx, leg.to_village_id, order.resource_id, leg.quantity).await?;
    }
    orders::complete_leg(&mut tx, leg).await?;
    tx.commit().await?;
    info!(order = %order.id, leg = leg.seq, "leg completed");
    Ok(())
}

async fn retry_or_fail(st: &AppState, leg: &Leg, status: &str, report: &mut TickReport) -> Result<()> {
    if leg.attempts + 1 < st.cfg.sim_leg_max_attempts {
        orders::retry_leg(&st.pool, leg.id, &format!("mission {status}")).await?;
        report.legs_retried += 1;
        return Ok(());
    }
    if let Some(hold) = leg.hold_id {
        st.clients.inventory.release_hold(hold).await?;
    }
    orders::close_order(&st.pool, leg.order_id, "failed", Some(&format!("mission {status} {} times", leg.attempts + 1))).await?;
    report.orders_failed += 1;
    Ok(())
}

/// Cancels an open order: cancels its published mission and releases its hold.
pub async fn cancel_order(st: &AppState, order_id: Uuid) -> Result<()> {
    let order = orders::get_order(&st.pool, order_id).await?.ok_or_else(|| AppError::NotFound(format!("order {order_id}")))?;
    if !orders::OPEN_STATUSES.contains(&order.status.as_str()) {
        return Err(AppError::conflict("ORDER_CLOSED", format!("order is {}", order.status)));
    }
    for leg in orders::order_legs(&st.pool, order_id).await? {
        if leg.status == "published" {
            if let Some(m) = leg.mission_id {
                if let Err(e) = st.clients.mission.cancel(m).await {
                    // Already completed or cancelled on the mission side.
                    if e.upstream_status() != Some(409) {
                        return Err(e);
                    }
                }
            }
        }
        if matches!(leg.status.as_str(), "held" | "published") {
            if let Some(hold) = leg.hold_id {
                st.clients.inventory.release_hold(hold).await?;
            }
        }
    }
    orders::close_order(&st.pool, order_id, "cancelled", None).await
}
