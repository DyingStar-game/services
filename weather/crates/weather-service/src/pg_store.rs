//! PostgreSQL (CNPG) storage of the atmosphere model's checkpoints: one row per (body, slot) in
//! `atmosphere_checkpoint` (migration embedded, applied at start). The model runs on plain threads
//! (not on the tokio runtime): they call this store, which runs each query on the runtime through
//! its handle (`Handle::block_on`) and waits, without ever blocking a runtime thread.

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::io;
use tokio::runtime::Handle;
use tracing::{info, warn};
use weather_atmosphere::live::{CheckpointMeta, StateStore, StoredState};

#[derive(Clone)]
pub struct PgStateStore {
    pool: PgPool,
    rt: Handle,
}

fn db_err(e: sqlx::Error) -> io::Error {
    io::Error::other(format!("database: {e}"))
}

impl PgStateStore {
    /// Connects (retrying while PostgreSQL is not ready, as in a fresh cluster) and applies the
    /// embedded migrations. Must be called on the tokio runtime.
    pub async fn connect(database_url: &str) -> anyhow::Result<PgStateStore> {
        // Never log the credentials of the URL.
        let target = database_url.rsplit('@').next().unwrap_or_default();
        info!(target, "atmosphere state: connecting to PostgreSQL");
        let max_attempts = 30;
        let mut attempt = 1;
        let pool = loop {
            match PgPoolOptions::new().max_connections(4).connect(database_url).await {
                Ok(p) => break p,
                Err(e) if attempt < max_attempts => {
                    warn!(attempt, max_attempts, error = %e, "PostgreSQL not ready, retrying in 5 s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    attempt += 1;
                }
                Err(e) => return Err(e.into()),
            }
        };
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(PgStateStore { pool, rt: Handle::current() })
    }
}

impl StateStore for PgStateStore {
    fn save(&mut self, body: &str, slot: usize, bytes: &[u8], meta: &CheckpointMeta) -> io::Result<String> {
        let key = body.to_lowercase();
        let meta_json = serde_json::to_string(meta).map_err(io::Error::other)?;
        let q = sqlx::query(
            "INSERT INTO atmosphere_checkpoint (body, slot, model_time, healthy, saved_at, meta, state)
             VALUES ($1, $2, $3, $4, now(), $5::jsonb, $6)
             ON CONFLICT (body, slot) DO UPDATE SET model_time = EXCLUDED.model_time, healthy = EXCLUDED.healthy,
                 saved_at = EXCLUDED.saved_at, meta = EXCLUDED.meta, state = EXCLUDED.state",
        )
        .bind(&key)
        .bind((slot % 2) as i16)
        .bind(meta.model_time_s)
        .bind(meta.healthy)
        .bind(meta_json)
        .bind(bytes)
        .execute(&self.pool);
        // One statement: the row is replaced atomically, old or new, never a mix.
        self.rt.block_on(q).map_err(db_err)?;
        Ok(format!("atmosphere_checkpoint({key}, {})", slot % 2))
    }

    fn load_latest_healthy(&self, body: &str) -> io::Result<Vec<StoredState>> {
        let key = body.to_lowercase();
        let q = sqlx::query("SELECT slot, meta::text AS meta, state FROM atmosphere_checkpoint WHERE body = $1 AND healthy").bind(&key).fetch_all(&self.pool);
        let rows = self.rt.block_on(q).map_err(db_err)?;
        let mut out = Vec::new();
        for r in rows {
            let slot: i16 = r.try_get("slot").map_err(db_err)?;
            let meta_txt: String = r.try_get("meta").map_err(db_err)?;
            let Ok(meta) = serde_json::from_str::<CheckpointMeta>(&meta_txt) else {
                warn!(body = %key, slot, "atmosphere checkpoint with unreadable metadata, skipped");
                continue;
            };
            let bytes: Vec<u8> = r.try_get("state").map_err(db_err)?;
            out.push(StoredState { bytes, meta, slot: slot as usize, location: format!("atmosphere_checkpoint({key}, {slot})") });
        }
        out.sort_by(|a, b| b.meta.chunk.cmp(&a.meta.chunk));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(chunk: u64, healthy: bool) -> CheckpointMeta {
        CheckpointMeta {
            body: "TestBody".into(),
            chunk,
            model_time_s: chunk as f64 * 3600.0,
            written_unix: 1.0,
            healthy,
            spun_up: true,
            unix_offset_s: 1.7e9,
        }
    }

    /// Needs a PostgreSQL: `TEST_DATABASE_URL=postgres://postgres:x@localhost:55432/postgres
    /// cargo test -p weather-service -- --ignored`. Not run in the CI (no database there).
    #[test]
    #[ignore = "needs TEST_DATABASE_URL (a PostgreSQL)"]
    fn postgres_store_round_trip() {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
            eprintln!("TEST_DATABASE_URL unset: skipped");
            return;
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Called from this plain thread, as the model's threads do.
        let mut store = rt.block_on(PgStateStore::connect(&url)).expect("connect");
        let body = format!("test_body_{}", std::process::id());
        let a: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let b: Vec<u8> = vec![7; 1000];
        store.save(&body, 0, &a, &meta(3, true)).unwrap();
        store.save(&body, 1, &b, &meta(6, true)).unwrap();
        let got = store.load_latest_healthy(&body).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].meta.chunk, got[0].slot), (6, 1));
        assert_eq!(got[0].meta, meta(6, true));
        assert_eq!(got[0].bytes, b);
        assert_eq!(got[1].bytes, a);
        // A slot rewritten as unhealthy is no longer offered; the other one is.
        store.save(&body, 1, &b, &meta(9, false)).unwrap();
        let got = store.load_latest_healthy(&body).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].meta.chunk, 3);
        // The migration is idempotent: a second connection applies nothing new.
        rt.block_on(PgStateStore::connect(&url)).expect("reconnect");
        rt.block_on(sqlx::query("DELETE FROM atmosphere_checkpoint WHERE body = $1").bind(&body).execute(&store.pool)).unwrap();
    }
}
