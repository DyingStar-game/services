//! PostgreSQL access: one module per table family. Queries are plain
//! `sqlx::query` / `query_as` (no compile-time checking, so the crate builds
//! without a database).

pub mod connection;
pub mod events;
pub mod npcs;
pub mod patterns;
pub mod settings;
pub mod tasks;
pub mod workplaces;

#[cfg(test)]
pub mod testdb;
#[cfg(test)]
mod it;
