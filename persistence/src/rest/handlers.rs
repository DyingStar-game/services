use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    db::queries::{delete_item_by_uuid, get_all_items, get_item_by_uuid, upsert_single_item},
    error::AppError,
    websocket::{
        messages::{BridgeEventEnvelope, GenericPropsRequest, Item, PutItemRequest, Vec3},
        server::AppState,
    },
};

// ─── Shared helper ──────────────────────────────────────────────────────────

/// Recompose an `Item` into the canonical GenericPropsRequest-shaped JSON:
/// `{ object_type, object_uuid, object_data: { ...rest, parent_id?, scenename?, position?, rotation? } }`
pub fn item_to_json(item: &Item) -> Value {
    let mut data_map: Map<String, Value> = item
        .object_data
        .first()
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    if let Some(ref v) = item.parent_id {
        data_map.insert("parent_id".to_string(), serde_json::json!(v));
    }
    if let Some(ref v) = item.scenename {
        data_map.insert("scenename".to_string(), serde_json::json!(v));
    }
    if let Some(ref v) = item.position {
        data_map.insert("position".to_string(), serde_json::json!(v));
    }
    if let Some(ref v) = item.rotation {
        data_map.insert("rotation".to_string(), serde_json::json!(v));
    }

    serde_json::json!({
        "object_type": item.object_type,
        "object_uuid": item.uuid,
        "object_data": Value::Object(data_map),
    })
}

// ─── Query parameters ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GetItemsParams {
    pub object_type: Option<String>,
    pub parent_id: Option<String>,
    pub scenename: Option<String>,
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page() -> usize {
    1
}
fn default_page_size() -> usize {
    100
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// POST /items
/// Create a new item. uuid is provided in the request body as `object_uuid`.
pub async fn create_item(
    State(state): State<AppState>,
    Json(body): Json<GenericPropsRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut data_map = body
        .object_data
        .as_object()
        .cloned()
        .unwrap_or_default();

    let parent_id = data_map
        .remove("parent_id")
        .and_then(|v| v.as_str().map(str::to_owned));
    let scenename = data_map
        .remove("scenename")
        .and_then(|v| v.as_str().map(str::to_owned));
    let position: Option<Vec3> = data_map
        .remove("position")
        .and_then(|v| serde_json::from_value(v).ok());
    let rotation: Option<Vec3> = data_map
        .remove("rotation")
        .and_then(|v| serde_json::from_value(v).ok());

    let item = Item {
        uuid: body.object_uuid,
        object_type: body.object_type,
        object_data: vec![Value::Object(data_map)],
        parent_id,
        scenename,
        position,
        rotation,
    };

    upsert_single_item(&state.session, &state.queries, &item).await?;
    state.cache.insert(item.clone());

    // Notify all active WebSocket connections.
    let envelope = BridgeEventEnvelope {
        event_type: "plugin".to_string(),
        namespace: Some("genericprops".to_string()),
        name: "create_object".to_string(),
        payload: item_to_json(&item),
    };
    if let Ok(text) = serde_json::to_string(&envelope) {
        // Ignore send errors — no connected clients is fine.
        let _ = state.broadcaster.send(text);
    }

    Ok((StatusCode::CREATED, Json(item_to_json(&item))))
}

/// GET /items
/// Search all items with optional filters and pagination.
pub async fn get_items(
    State(state): State<AppState>,
    Query(params): Query<GetItemsParams>,
) -> Result<impl IntoResponse, AppError> {
    let all = get_all_items(&state.session, &state.config.scylla_keyspace).await?;

    let filtered: Vec<&Item> = all
        .iter()
        .filter(|item| {
            params
                .object_type
                .as_deref()
                .map_or(true, |f| item.object_type == f)
                && params
                    .parent_id
                    .as_deref()
                    .map_or(true, |f| item.parent_id.as_deref() == Some(f))
                && params
                    .scenename
                    .as_deref()
                    .map_or(true, |f| item.scenename.as_deref() == Some(f))
        })
        .collect();

    let total = filtered.len();
    let page_size = params.page_size.max(1);
    let page = params.page.max(1);
    let skip = (page - 1) * page_size;

    let items: Vec<Value> = filtered
        .into_iter()
        .skip(skip)
        .take(page_size)
        .map(item_to_json)
        .collect();

    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "page_size": page_size,
    })))
}

/// GET /items/:uuid
/// Return a single item or 404.
pub async fn get_item(
    State(state): State<AppState>,
    Path(uuid): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    match get_item_by_uuid(&state.session, &state.queries, &uuid).await? {
        Some(item) => Ok(Json(item_to_json(&item))),
        None => Err(AppError::NotFound(format!("item '{uuid}' not found"))),
    }
}

/// PUT /items/:uuid
/// Replace item data. Writes through to DB and cache.
pub async fn put_item(
    State(state): State<AppState>,
    Path(uuid): Path<String>,
    Json(body): Json<PutItemRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut data_map = body
        .object_data
        .as_object()
        .cloned()
        .unwrap_or_default();

    let parent_id = data_map
        .remove("parent_id")
        .and_then(|v| v.as_str().map(str::to_owned));
    let scenename = data_map
        .remove("scenename")
        .and_then(|v| v.as_str().map(str::to_owned));
    let position: Option<Vec3> = data_map
        .remove("position")
        .and_then(|v| serde_json::from_value(v).ok());
    let rotation: Option<Vec3> = data_map
        .remove("rotation")
        .and_then(|v| serde_json::from_value(v).ok());

    let item = Item {
        uuid,
        object_type: body.object_type,
        object_data: vec![Value::Object(data_map)],
        parent_id,
        scenename,
        position,
        rotation,
    };

    upsert_single_item(&state.session, &state.queries, &item).await?;
    state.cache.insert(item.clone());

    // Notify all active WebSocket connections.
    let envelope = BridgeEventEnvelope {
        event_type: "plugin".to_string(),
        namespace: Some("genericprops".to_string()),
        name: "update_object_from_external".to_string(),
        payload: item_to_json(&item),
    };
    if let Ok(text) = serde_json::to_string(&envelope) {
        let _ = state.broadcaster.send(text);
    }

    Ok(Json(item_to_json(&item)))
}

/// DELETE /items/:uuid
/// Remove item from DB and cache. Always returns 204.
pub async fn delete_item(
    State(state): State<AppState>,
    Path(uuid): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    delete_item_by_uuid(&state.session, &state.queries, &uuid).await?;
    state.cache.remove(&uuid);
    Ok(StatusCode::NO_CONTENT)
}
