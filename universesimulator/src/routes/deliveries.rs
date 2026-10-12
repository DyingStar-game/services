//! Off-mission warehouse deliveries from the game server (ADR 0012).

use axum::{extract::State, http::StatusCode, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::{Caller, ROLE_DELIVER};
use crate::clients::Holder;
use crate::db::{catalog, history::{self, Delivery, NewDelivery}, world};
use crate::error::{AppError, Result};
use crate::simulation::stocks;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarehouseRef {
    pub id: Option<Uuid>,
    pub poi_uuid: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceRef {
    pub code: Option<String>,
    pub good_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deliverer {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryInput {
    pub idempotency_key: String,
    pub warehouse: WarehouseRef,
    pub resource: ResourceRef,
    pub quantity: i64,
    pub deliverer: Deliverer,
    #[serde(default)]
    pub metadata: Value,
}

/// Fingerprint of the request content (everything but the key and the metadata).
fn fingerprint(warehouse: Uuid, resource: Uuid, quantity: i64, deliverer: &Deliverer) -> String {
    let mut h = Sha256::new();
    h.update(format!("{warehouse}|{resource}|{quantity}|{}|{}", deliverer.kind, deliverer.id));
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn response(d: &Delivery) -> Value {
    json!({
        "delivery": d,
        "acceptedQuantity": d.accepted_quantity,
        "rejectedQuantity": d.requested_quantity - d.accepted_quantity,
        "stock": d.stock_after,
    })
}

pub async fn create(State(st): State<AppState>, Extension(caller): Extension<Caller>, Json(body): Json<DeliveryInput>) -> Result<(StatusCode, Json<Value>)> {
    caller.require_role(ROLE_DELIVER)?;
    if body.idempotency_key.is_empty() || body.idempotency_key.len() > 128 {
        return Err(AppError::BadRequest("idempotencyKey must have 1 to 128 characters".into()));
    }
    if !(1..=10_000_000_000_000).contains(&body.quantity) {
        return Err(AppError::BadRequest("quantity must be between 1 and 1e13".into()));
    }
    let deliverer = Holder::of(&body.deliverer.kind, body.deliverer.id)
        .filter(|h| h.holder_type != "warehouse")
        .ok_or_else(|| AppError::BadRequest("deliverer.type must be player or npc".into()))?;
    let warehouse_id = body.warehouse.id.or(body.warehouse.poi_uuid).ok_or_else(|| AppError::BadRequest("warehouse.id or warehouse.poiUuid is required".into()))?;
    let warehouse = world::get_warehouse(&st.pool, warehouse_id).await?.filter(|w| w.active).ok_or_else(|| AppError::NotFound(format!("warehouse {warehouse_id}")))?;
    let resource = catalog::find_resource(&st.pool, body.resource.code.as_deref(), body.resource.good_type.as_deref())
        .await?
        .ok_or_else(|| AppError::NotFound("resource".into()))?;
    let hash = fingerprint(warehouse.id, resource.id, body.quantity, &body.deliverer);

    // Replay of a finished delivery.
    let existing = history::get_delivery(&st.pool, &body.idempotency_key).await?;
    if let Some(d) = &existing {
        if d.request_hash != hash {
            return Err(AppError::conflict("IDEMPOTENCY_CONFLICT", "idempotencyKey already used for another delivery"));
        }
        if d.status == "done" {
            return Ok((StatusCode::OK, Json(response(d))));
        }
    }

    let level = stocks::refresh_one(&st, warehouse.id, resource.id, &resource.inventory_good_type).await?;
    let capacity = world::get_capacity(&st.pool, warehouse.id, resource.id).await?;
    let accepted = capacity.map(|c| (c - level.quantity).clamp(0, body.quantity)).unwrap_or(body.quantity);

    let delivery = match existing {
        Some(d) => {
            history::reopen_delivery(&st.pool, d.id, accepted).await?;
            d
        }
        None => {
            let metadata = if body.metadata.is_null() { json!({}) } else { body.metadata.clone() };
            let new = NewDelivery {
                key: &body.idempotency_key,
                hash: &hash,
                warehouse: warehouse.id,
                resource: resource.id,
                requested: body.quantity,
                accepted,
                deliverer_type: deliverer.holder_type,
                deliverer_id: deliverer.holder_id,
                metadata: &metadata,
            };
            // A concurrent request with the same key won the insert.
            history::insert_delivery(&st.pool, &new)
                .await?
                .ok_or_else(|| AppError::conflict("DELIVERY_IN_PROGRESS", "a delivery with this idempotencyKey is in progress"))?
        }
    };

    if accepted > 0 {
        if let Err(e) = st.clients.inventory.transfer(deliverer, Holder::warehouse(warehouse.id), &resource.inventory_good_type, accepted).await {
            let failed = history::finish_delivery(&st.pool, delivery.id, "failed", 0, None, Some(&e.to_string())).await?;
            if e.upstream_code() == Some("INSUFFICIENT_GOODS") {
                return Err(AppError::conflict("INSUFFICIENT_STOCK", format!("the deliverer does not hold {accepted} {}", resource.code)));
            }
            tracing::warn!(delivery = %failed.id, error = %e, "delivery transfer failed");
            return Err(e);
        }
    }
    let after = stocks::refresh_one(&st, warehouse.id, resource.id, &resource.inventory_good_type).await?;
    let done = history::finish_delivery(&st.pool, delivery.id, "done", accepted, Some(after.quantity), None).await?;
    Ok((StatusCode::CREATED, Json(response(&done))))
}
