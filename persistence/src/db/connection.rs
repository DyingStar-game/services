use anyhow::Context;
use scylla::client::session::Session;
use scylla::client::session_builder::SessionBuilder;
use tracing::info;

/// Build a ScyllaDB session and ensure the keyspace + table exist.
pub async fn connect(
    nodes: &[String],
    keyspace: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<Session> {
    info!("Connecting to ScyllaDB nodes: {:?}", nodes);

    let mut builder = SessionBuilder::new().known_nodes(nodes);
    if let (Some(user), Some(pass)) = (username, password) {
        builder = builder.user(user, pass);
    }
    let session = builder
        .build()
        .await
        .context("Failed to build ScyllaDB session")?;

    info!("Connected to ScyllaDB");

    // Create keyspace (NetworkTopologyStrategy with RF=1 for dev;
    // adjust replication_factor via CQL ALTER for production clusters).
    session
        .query_unpaged(
            format!(
                "CREATE KEYSPACE IF NOT EXISTS {keyspace} \
                 WITH replication = {{'class': 'NetworkTopologyStrategy', 'replication_factor': 1}}"
            ),
            (),
        )
        .await
        .context("Failed to create keyspace")?;

    // Create items table.
    // `data` is stored as TEXT (JSON-serialised Vec<serde_json::Value>).
    // `position` and `rotation` are stored as TEXT (JSON-serialised Vec3).
    session
        .query_unpaged(
            format!(
                "CREATE TABLE IF NOT EXISTS {keyspace}.items ( \
                    uuid       TEXT PRIMARY KEY, \
                    object_def TEXT, \
                    data       TEXT, \
                    parent_id  TEXT, \
                    scenename  TEXT, \
                    position   TEXT, \
                    rotation   TEXT \
                )"
            ),
            (),
        )
        .await
        .context("Failed to create items table")?;

    info!("Keyspace '{keyspace}' and table 'items' are ready");
    Ok(session)
}
