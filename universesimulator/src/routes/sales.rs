//! Off-mission sales in a warehouse (ADR 0017): a player shelves goods of their own that no mission of
//! theirs counted (the mission service tells the game server what it counted); the issuer buys the rest
//! at the market price, from its treasury, and tells the player. It buys what the warehouse had room
//! for: past its capacity the goods stay on the shelf, not paid, and the player is told how much.
//!
//! A sale is recorded BEFORE it is paid, with its price: a failure anywhere after that is resumed by
//! replaying the same key, which pays what is left to pay and never twice.

use axum::{extract::State, http::StatusCode, Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{Caller, ROLE_DELIVER};
use crate::db::{catalog::{self, Federation}, history::{self, NewSale, Sale}, world};
use crate::error::{AppError, Result};
use crate::simulation::{stocks, treasury};
use crate::state::AppState;

/// Longest idempotency key: the economy's limit (128) less the suffix of a payment's leg.
pub const MAX_KEY_LEN: usize = 100;
/// The economy's largest movement.
pub const MAX_AMOUNT: i64 = 10_000_000_000_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaleInput {
    /// Unique per sale: the same key replays the same answer and pays once.
    pub idempotency_key: String,
    /// The warehouse (its id: the cargo depot's uuid).
    pub warehouse_id: Uuid,
    /// The good shelved, as inventory names it (`ore.gold`).
    pub good_type: String,
    /// What is left of the drop once the player's missions counted their part.
    pub quantity: i64,
    pub player_id: Uuid,
}

/// The price of [quantity] at [unit_price], rounded; nothing for nothing sold or no price.
pub fn amount_for(quantity: i64, unit_price: f64) -> i64 {
    if quantity <= 0 || unit_price <= 0.0 {
        return 0;
    }
    ((quantity as f64) * unit_price).round() as i64
}

/// What of [quantity] just shelved a warehouse buys: what fitted under its [capacity] for the resource
/// before the drop, all of it without a capacity. The goods are on the shelf already, so [stock] holds
/// them.
pub fn room_for(capacity: Option<i64>, stock: i64, quantity: i64) -> i64 {
    let Some(capacity) = capacity else { return quantity.max(0) };
    let before = (stock - quantity).max(0);
    (capacity - before).clamp(0, quantity.max(0))
}

fn response(s: &Sale) -> Value {
    json!({
        "sale": s, "quantity": s.quantity, "bought": s.bought, "refused": s.quantity - s.bought,
        "unitPrice": s.unit_price, "amount": s.amount, "currency": s.currency, "paid": s.paid_at.is_some(),
    })
}

pub async fn create(State(st): State<AppState>, Extension(caller): Extension<Caller>, Json(body): Json<SaleInput>) -> Result<(StatusCode, Json<Value>)> {
    caller.require_role(ROLE_DELIVER)?;
    if body.idempotency_key.is_empty() || body.idempotency_key.len() > MAX_KEY_LEN {
        return Err(AppError::BadRequest(format!("idempotencyKey must have 1 to {MAX_KEY_LEN} characters")));
    }
    if body.quantity < 1 {
        return Err(AppError::BadRequest("quantity must be at least 1".into()));
    }
    let (sale, created) = sell(&st, &body).await?;
    Ok((if created { StatusCode::CREATED } else { StatusCode::OK }, Json(response(&sale))))
}

/// Records the sale [input] asks for (or finds the one its key already made), pays what of it is not
/// paid yet and tells the player. Returns the sale and whether this call made it.
pub async fn sell(st: &AppState, input: &SaleInput) -> Result<(Sale, bool)> {
    let warehouse = world::get_warehouse(&st.pool, input.warehouse_id).await?.filter(|w| w.active).ok_or_else(|| AppError::NotFound(format!("warehouse {}", input.warehouse_id)))?;
    let village = world::get_village(&st.pool, warehouse.village_id).await?.ok_or_else(|| AppError::NotFound("village".into()))?;
    let fed = catalog::get_federation(&st.pool, village.federation_id).await?.ok_or_else(|| AppError::NotFound("federation".into()))?;
    let resource = catalog::find_resource(&st.pool, None, Some(&input.good_type)).await?.ok_or_else(|| AppError::NotFound(format!("resource {}", input.good_type)))?;

    let (sale, created) = match history::get_sale(&st.pool, &input.idempotency_key).await? {
        Some(sale) => (sale, false),
        None => {
            // The market price (the issuer's index, as the town hall's board shows it), else the base price.
            let unit_price = history::price_indices(&st.pool, fed.id).await?.into_iter().find(|p| p.resource_id == resource.id).map_or(resource.base_price, |p| p.unit_price);
            // Read where the goods are, not from the last tick's snapshot: a warehouse fills in minutes.
            let level = stocks::refresh_one(st, warehouse.id, resource.id, &resource.inventory_good_type).await?;
            let capacity = world::get_capacity(&st.pool, warehouse.id, resource.id).await?;
            let bought = room_for(capacity, level.quantity, input.quantity);
            let amount = amount_for(bought, unit_price);
            if amount > MAX_AMOUNT {
                return Err(AppError::BadRequest("the sale is worth more than one payment may carry".into()));
            }
            let new = NewSale { key: &input.idempotency_key, player: input.player_id, warehouse: warehouse.id, resource: resource.id, quantity: input.quantity, bought, unit_price, amount, currency: &fed.currency };
            match history::insert_sale(&st.pool, &new).await? {
                Some(sale) => (sale, true),
                // A concurrent request with the same key recorded it first.
                None => (history::get_sale(&st.pool, &input.idempotency_key).await?.ok_or_else(|| AppError::conflict("SALE_IN_PROGRESS", "a sale with this key is in progress"))?, false),
            }
        }
    };
    if sale.player_id != input.player_id || sale.warehouse_id != warehouse.id || sale.resource_id != resource.id || sale.quantity != input.quantity {
        return Err(AppError::conflict("IDEMPOTENCY_CONFLICT", "idempotencyKey already used for another sale"));
    }
    if sale.paid_at.is_some() {
        return Ok((sale, created));
    }

    if sale.amount > 0 {
        pay(st, &fed, &sale, &format!("sale:{}:{}", village.name, resource.code)).await?;
    }
    // Only the request that marks it paid tells the player: a replay racing it says nothing twice.
    if history::mark_sale_paid(&st.pool, &sale.idempotency_key).await? {
        let notice = json!({
            "type": "warehouse.sale",
            "data": {
                "village": village.name, "goodType": resource.inventory_good_type, "name": resource.name,
                "nameFr": resource.name_fr, "quantity": sale.bought, "refused": sale.quantity - sale.bought,
                "unit": resource.unit, "amount": sale.amount, "currency": sale.currency,
            },
        });
        // Best effort: the money is paid whether the player hears of it or not.
        if let Err(e) = st.clients.social.notify_player(sale.player_id, &notice).await {
            tracing::warn!(player = %sale.player_id, error = %e, "sale notification not sent");
        }
        tracing::info!(player = %sale.player_id, village = %village.name, resource = %resource.code, quantity = sale.quantity, bought = sale.bought, amount = sale.amount, "off-mission sale");
    }
    let paid = history::get_sale(&st.pool, &sale.idempotency_key).await?.unwrap_or(sale);
    Ok((paid, created))
}

/// Moves [sale]'s amount from [fed]'s treasury to the player: the debit, then the credit, each done
/// once for the sale's key whatever was replayed. A treasury that cannot pay mints first, as it does
/// for a mission's escrow (ADR 0009), and tries once more.
async fn pay(st: &AppState, fed: &Federation, sale: &Sale, reference: &str) -> Result<()> {
    let economie = &st.clients.economie;
    let key = &sale.idempotency_key;
    let debit = economie.debit_treasury(fed.political_entity_id, sale.amount, &sale.currency, key, reference).await;
    if let Err(e) = debit {
        if e.upstream_code() != Some("INSUFFICIENT_FUNDS") {
            return Err(e);
        }
        treasury::mint(st, fed, "sale refused: treasury insufficient").await?;
        economie.debit_treasury(fed.political_entity_id, sale.amount, &sale.currency, key, reference).await?;
    }
    economie.credit_player(sale.player_id, sale.amount, &sale.currency, key, reference).await
}

#[cfg(test)]
mod tests {
    use super::amount_for;

    #[test]
    fn a_warehouse_buys_what_it_had_room_for() {
        assert_eq!(super::room_for(None, 9_000, 40), 40, "no capacity: all of it");
        assert_eq!(super::room_for(Some(5_000), 1_040, 40), 40, "room for it all");
        assert_eq!(super::room_for(Some(5_000), 5_030, 40), 10, "ten fitted");
        assert_eq!(super::room_for(Some(5_000), 8_367, 40), 0, "full before the drop");
        assert_eq!(super::room_for(Some(5_000), 0, 40), 40, "a stock not read yet: never less than nothing");
    }

    #[test]
    fn a_sale_is_worth_its_price_rounded() {
        assert_eq!(amount_for(120, 12.5), 1500);
        assert_eq!(amount_for(3, 0.1), 0, "less than half a credit: nothing, not a credit a crate");
        assert_eq!(amount_for(5, 0.1), 1);
        assert_eq!(amount_for(0, 12.5), 0);
        assert_eq!(amount_for(10, 0.0), 0, "no price: nothing");
    }
}
