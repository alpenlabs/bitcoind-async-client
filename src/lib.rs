//! BitcoinD JSON-RPC Async Client
//!
//! # Features
//!
//! - `29_0`: Enable support for Bitcoin Core v29
//!
//! # Usage
//!
//! ```rust
//! use bitcoind_async_client::Client;
//!
//! let client = Client::new("http://localhost:8332", "username", "password", None, None).await?;
//!
//! let blockchain_info = client.get_blockchain_info().await?;
//! ```
//!
//! # License
//!
//! This work is dual-licensed under MIT and Apache 2.0.
//! You can choose between one of them if you use this work.

pub mod client;
pub mod error;
pub mod traits;

// TODO: remove this once upstreamed PRs are merged
pub mod types;

pub use client::*;

// v29
#[cfg(feature = "29_0")]
pub use client::v29;

#[cfg(test)]
pub mod test_utils;
