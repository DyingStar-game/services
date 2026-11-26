use anyhow::Context;
use futures::StreamExt;
use scylla::client::session::Session;
use scylla::statement::prepared::PreparedStatement;
use scylla::statement::unprepared::Statement;
use tracing::{debug, warn};

use crate::websocket::messages::{Item, Vec3};

/// Prepared statements bundled together for reuse.
pub struct Queries {
    pub upsert_item: PreparedStatement,
    pub get_item: PreparedStatement,
    pub delete_item: PreparedStatement,
}

impl Queries {
    pub async fn prepare(session: &Session, keyspace: &str) -> anyhow::Result<Self> {
        let upsert_item = session
            .prepare(format!(
                "INSERT INTO {keyspace}.items \
                 (uuid, object_def, data, parent_id, scenename, position, rotation) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)"
            ))
            .await
            .context("Failed to prepare upsert_item statement")?;

        let get_item = session
            .prepare(format!(
                "SELECT uuid, object_def, data, parent_id, scenename, position, rotation \
                 FROM {keyspace}.items WHERE uuid = ?"
            ))
            .await
            .context("Failed to prepare get_item statement")?;

        let delete_item = session
            .prepare(format!("DELETE FROM {keyspace}.items WHERE uuid = ?"))
            .await
            .context("Failed to prepare delete_item statement")?;

        Ok(Self { upsert_item, get_item, delete_item })
    }
}

/// Rows ScyllaDB returns per page when every item is read.
pub const ITEMS_PAGE_SIZE: i32 = 5000;

type ItemRow = (String, String, String, Option<String>, Option<String>, Option<String>, Option<String>);

/// Every item of the database as a stream, read [`ITEMS_PAGE_SIZE`] rows at a time: only the
/// current page is held in memory. Rows that cannot be read are skipped with a warning.
pub async fn stream_all_items(
    session: &Session,
    keyspace: &str,
) -> anyhow::Result<impl futures::Stream<Item = Item> + Send + Unpin> {
    let statement = Statement::new(format!(
        "SELECT uuid, object_def, data, parent_id, scenename, position, rotation FROM {keyspace}.items"
    ))
    .with_page_size(ITEMS_PAGE_SIZE);
    let rows_stream = session
        .query_iter(statement, ())
        .await
        .context("Failed to start query_iter")?
        .rows_stream::<ItemRow>()
        .context("Failed to create typed rows stream")?;
    Ok(rows_stream.filter_map(|row| async move {
        match row {
            Ok(row) => row_to_item(row),
            Err(e) => {
                warn!("Row deserialisation error: {e}");
                None
            }
        }
    }).boxed())
}

/// One database row as an item, None (with a warning) when its data is not valid JSON.
fn row_to_item(row: ItemRow) -> Option<Item> {
    let (uuid, object_def, data_json, parent_id, scenename, position_json, rotation_json) = row;
    match serde_json::from_str(&data_json) {
        Ok(object_data) => {
            let position: Option<Vec3> = position_json.as_deref().and_then(|s| serde_json::from_str(s).ok());
            let rotation: Option<Vec3> = rotation_json.as_deref().and_then(|s| serde_json::from_str(s).ok());
            Some(Item { uuid, object_type: object_def, object_data, parent_id, scenename, position, rotation })
        }
        Err(e) => {
            warn!("Failed to deserialise data for item {uuid}: {e}");
            None
        }
    }
}

/// Every item collected into a `Vec<Item>` (the REST search). The WebSocket `get_all_items`
/// streams instead ([`stream_all_items`]): collecting ~20 000 items killed the service at 128Mi.
pub async fn get_all_items(session: &Session, keyspace: &str) -> anyhow::Result<Vec<Item>> {
    let items: Vec<Item> = stream_all_items(session, keyspace).await?.collect().await;
    debug!("Fetched {} items from database", items.len());
    Ok(items)
}

/// Fetch a single item by UUID. Returns `None` if not found.
pub async fn get_item_by_uuid(
    session: &Session,
    queries: &Queries,
    uuid: &str,
) -> anyhow::Result<Option<Item>> {
    let mut rows_stream = session
        .execute_iter(queries.get_item.clone(), (uuid,))
        .await
        .context("Failed to execute get_item")?
        .rows_stream::<(String, String, String, Option<String>, Option<String>, Option<String>, Option<String>)>()
        .context("Failed to create typed rows stream")?;

    if let Some(row) = rows_stream.next().await {
        let (uuid, object_def, data_json, parent_id, scenename, position_json, rotation_json) =
            row.context("Failed to deserialise get_item row")?;
        let object_data = serde_json::from_str(&data_json)
            .context("Failed to deserialise object_data")?;
        let position: Option<Vec3> =
            position_json.as_deref().and_then(|s| serde_json::from_str(s).ok());
        let rotation: Option<Vec3> =
            rotation_json.as_deref().and_then(|s| serde_json::from_str(s).ok());
        Ok(Some(Item { uuid, object_type: object_def, object_data, parent_id, scenename, position, rotation }))
    } else {
        Ok(None)
    }
}

/// Upsert a single item to ScyllaDB.
pub async fn upsert_single_item(
    session: &Session,
    queries: &Queries,
    item: &Item,
) -> anyhow::Result<()> {
    let data_json = serde_json::to_string(&item.object_data)
        .context("Failed to serialise object_data")?;
    let position_json = item.position.as_ref().and_then(|v| serde_json::to_string(v).ok());
    let rotation_json = item.rotation.as_ref().and_then(|v| serde_json::to_string(v).ok());
    session
        .execute_unpaged(
            &queries.upsert_item,
            (
                item.uuid.as_str(),
                item.object_type.as_str(),
                data_json,
                item.parent_id.as_deref(),
                item.scenename.as_deref(),
                position_json,
                rotation_json,
            ),
        )
        .await
        .context("Failed to upsert item")?;
    Ok(())
}

/// Delete an item by UUID from ScyllaDB.
pub async fn delete_item_by_uuid(
    session: &Session,
    queries: &Queries,
    uuid: &str,
) -> anyhow::Result<()> {
    session
        .execute_unpaged(&queries.delete_item, (uuid,))
        .await
        .context("Failed to delete item")?;
    Ok(())
}

/// Write a batch of items to ScyllaDB using pipelined prepared statements.
///
/// We avoid LOGGED/UNLOGGED BATCH CQL because each item has its own partition
/// key (uuid); pipelined individual inserts are more efficient in that case.
pub async fn batch_upsert_items(
    session: &Session,
    queries: &Queries,
    items: Vec<Item>,
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }

    debug!("Flushing {} items to ScyllaDB", items.len());

    // Fire all inserts concurrently and collect results.
    let futures: Vec<_> = items
        .iter()
        .map(|item| {
            let data_json = serde_json::to_string(&item.object_data)
                .unwrap_or_else(|_| "[]".to_string());
            let position_json = item.position.as_ref()
                .and_then(|v| serde_json::to_string(v).ok());
            let rotation_json = item.rotation.as_ref()
                .and_then(|v| serde_json::to_string(v).ok());
            session.execute_unpaged(
                &queries.upsert_item,
                (
                    item.uuid.as_str(),
                    item.object_type.as_str(),
                    data_json,
                    item.parent_id.as_deref(),
                    item.scenename.as_deref(),
                    position_json,
                    rotation_json,
                ),
            )
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    let errors: Vec<_> = results.into_iter().filter_map(|r| r.err()).collect();
    if !errors.is_empty() {
        warn!("{} item(s) failed to write to ScyllaDB", errors.len());
        for e in &errors {
            warn!("  write error: {e}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::row_to_item;

    fn row(data: &str, position: Option<&str>) -> super::ItemRow {
        ("u1".into(), "miningrock".into(), data.into(), Some("p1".into()), None, position.map(String::from), None)
    }

    #[test]
    fn a_row_becomes_an_item() {
        let item = row_to_item(row(r#"[{"hp": 3}]"#, Some(r#"{"x":1.0,"y":2.0,"z":3.0}"#))).expect("valid row");
        assert_eq!(item.uuid, "u1");
        assert_eq!(item.object_type, "miningrock");
        assert_eq!(item.parent_id.as_deref(), Some("p1"));
        assert_eq!(item.position.map(|p| p.y), Some(2.0));
        assert_eq!(item.object_data[0]["hp"], 3);
    }

    #[test]
    fn a_row_with_broken_data_is_skipped() {
        assert!(row_to_item(row("not json", None)).is_none());
    }
}
