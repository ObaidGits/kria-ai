//! Federated marketplace layer for the Capability Intelligence Layer
//! (design §8.2, §7.4).
//!
//! This module hosts the [`MarketplaceProvider`] trait — the pluggable seam for
//! multi-marketplace federation — and its first implementation,
//! [`ClawHubProvider`], which wraps the **frozen**
//! [`ClawHubClient`](crate::openclaw::clawhub::ClawHubClient) unchanged.
//!
//! Later tasks add `MarketIndex` (offline embedding into the `market_catalog`
//! derived table, task 6.2), incremental/concurrent sync with offline fallback
//! (task 6.3), and facade wiring (task 6.4). This task (6.1) provides only the
//! trait, the normalized [`MarketEntry`], and the ClawHub adapter + validation.

pub mod index;
pub mod provider;
#[cfg(test)]
mod reindex_pbt;

pub use index::{MarketCandidate, MarketIndex};
pub use provider::{ClawHubProvider, MarketEntry, MarketplaceProvider};
