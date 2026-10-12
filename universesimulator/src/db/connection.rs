use sqlx::{PgPool, postgres::PgPoolOptions};
use tracing::{info, warn};

use crate::config::Config;
use crate::error::Result;

/// Initialise a PostgreSQL connection pool and run any pending migrations.
/// Retries the connection up to 30 times with a 5-second delay so the app
/// survives being scheduled before PostgreSQL is ready (no init container needed).
pub async fn init(cfg: &Config) -> Result<PgPool> {
    // Never log the credentials of the URL.
    let target = cfg.database_url.rsplit('@').next().unwrap_or_default();
    info!(target, "connecting to PostgreSQL");

    let pool = connect_with_retry(&cfg.database_url).await?;

    info!("running database migrations");
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

async fn connect_with_retry(database_url: &str) -> Result<PgPool> {
    let max_attempts = 30;
    let delay = std::time::Duration::from_secs(5);

    for attempt in 1..=max_attempts {
        match PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
        {
            Ok(pool) => return Ok(pool),
            Err(e) if attempt < max_attempts => {
                warn!(attempt, max_attempts, error = %e, "PostgreSQL not ready, retrying in 5s");
                tokio::time::sleep(delay).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}
