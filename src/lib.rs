//! BitcoinD JSON-RPC Async Client
//!
//! # Features
//!
//! - `29_0`: Enable support for Bitcoin Core v29.
//! - `30_2`: Enable support for Bitcoin Core v30.2.
//! - `31_1`: Enable support for Bitcoin Core v31.1. This is enabled by default.
//!
//! The typed RPC traits require exactly one version feature. Clients using only
//! `call_raw` (the `raw_rpc` feature) can disable all version features. See
//! [Bitcoin Core versions](https://github.com/alpenlabs/bitcoind-async-client#bitcoin-core-versions)
//! and [Migrating to 0.16](https://github.com/alpenlabs/bitcoind-async-client#migrating-to-016).
//!
//! # Usage
//!
//! ```rust,no_run
//! use bitcoind_async_client::{traits::Reader, Auth, Client};
//!
//! # #[cfg(any(feature = "29_0", feature = "30_2", feature = "31_1"))]
//! # async fn example() -> Result<(), bitcoind_async_client::error::ClientError> {
//! let client = Client::new(
//!     "http://localhost:8332".to_string(),
//!     Auth::UserPass("username".to_string(), "password".to_string()),
//!     None,
//!     None,
//!     None,
//! )?;
//!
//! let blockchain_info = client.get_blockchain_info().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # License
//!
//! This work is dual-licensed under MIT and Apache 2.0.
//! You can choose between one of them if you use this work.

pub mod client;
pub mod error;
pub mod traits;

pub mod types;

pub use corepc_types;

pub use client::*;

#[cfg(test)]
pub mod test_utils;
