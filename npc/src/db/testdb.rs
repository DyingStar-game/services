//! A fresh, migrated database per test, created on the server named by
//! `NPC_TEST_DATABASE_URL` (e.g. `postgres://postgres:test@localhost:55432/postgres`).
//! Tests using it are skipped when the variable is unset.

use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub async fn fresh() -> Option<PgPool> {
    let Ok(base) = std::env::var("NPC_TEST_DATABASE_URL") else {
        eprintln!("NPC_TEST_DATABASE_URL unset; skipping database test");
        return None;
    };
    let admin = PgPool::connect(&base).await.expect("connect to NPC_TEST_DATABASE_URL");
    let name = format!("npc_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&admin).await.expect("create test database");
    let (server, _) = base.rsplit_once('/').expect("database URL with a path");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&format!("{server}/{name}"))
        .await
        .expect("connect to the test database");
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrations");
    Some(pool)
}
