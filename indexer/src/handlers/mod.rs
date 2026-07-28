//! The indexer's product-data HTTP API — everything that used to be a
//! direct Prisma query from `app/src/app/api/**` (search, votes, stakes,
//! ads, revenue, page views, users) now lives here instead, mirroring the
//! existing on-chain-read/tx-building endpoints in `src/api.rs`. See
//! `AGENTS.md` (root) for why: the database is owned by the indexer, and
//! that now extends to being the sole query layer for it too, not just
//! schema/population.
//!
//! Each submodule exposes `pub fn routes() -> axum::Router<Arc<ApiState>>`,
//! merged into the main router built in `src/api.rs`.

pub mod ads;
pub mod apps;
pub mod engine;
pub mod find;
pub mod platform;
pub mod revenue;
pub mod rewards;
pub mod stakes;
pub mod tags;
pub mod track;
pub mod users;
pub mod votes;
pub mod x402;
pub mod xp;
