//! Error types for the RPC client.
use std::fmt;

use bitcoin::Network;
use bitreq::Error as BitreqError;
use serde::{Deserialize, Serialize};
use serde_json::Error as SerdeJsonError;
use thiserror::Error;

/// Bitcoin Core `RPC_VERIFY_ERROR`, defined in
/// <https://github.com/bitcoin/bitcoin/blob/8f4a3ba8972dae9412ba975a040cea22c227f983/src/rpc/protocol.h#L47>.
const RPC_VERIFY_ERROR: i32 = -25;
/// Bitcoin Core `RPC_VERIFY_REJECTED`, defined in
/// <https://github.com/bitcoin/bitcoin/blob/8f4a3ba8972dae9412ba975a040cea22c227f983/src/rpc/protocol.h#L48>.
const RPC_VERIFY_REJECTED: i32 = -26;
/// Bitcoin Core `RPC_VERIFY_ALREADY_IN_UTXO_SET`, defined in
/// <https://github.com/bitcoin/bitcoin/blob/8f4a3ba8972dae9412ba975a040cea22c227f983/src/rpc/protocol.h#L49>.
const RPC_VERIFY_ALREADY_IN_UTXO_SET: i32 = -27;

/// The error type for errors produced in this library.
#[derive(Error, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientError {
    /// Missing username or password for the RPC server
    #[error("Missing username or password")]
    MissingUserPassword,

    /// RPC server returned an error
    ///
    /// # Note
    ///
    /// These errors are ABSOLUTELY UNDOCUMENTED.
    /// Check
    /// <https://github.com/bitcoin/bitcoin/blob/96b0a8f858ab24f3672360b8c830553b963de726/src/rpc/protocol.h#L24>
    /// and good luck!
    #[error("RPC server returned error '{1}' (code {0})")]
    Server(i32, String),

    /// Error parsing the RPC response, unlikely to be recoverable by retrying
    #[error("Error parsing rpc response: {0}")]
    Parse(String),

    /// Error creating the RPC request, retry might help
    #[error("Could not create RPC Param")]
    Param(String),

    /// Body error, unlikely to be recoverable by retrying
    #[error("{0}")]
    Body(String),

    /// HTTP status error, not retryable
    #[error("Obtained failure status({0}): {1}")]
    Status(u16, String),

    /// Error decoding the response, retry might not help
    #[error("Malformed Response: {0}")]
    MalformedResponse(String),

    /// Connection error, retry might help
    #[error("Could not connect: {0}")]
    Connection(String),

    /// Timeout error, retry might help
    #[error("Timeout")]
    Timeout,

    /// Redirect error, not retryable
    #[error("HttpRedirect: {0}")]
    HttpRedirect(String),

    /// Error building the request, unlikely to be recoverable
    #[error("Could not build request: {0}")]
    ReqBuilder(String),

    /// Maximum retries exceeded, not retryable
    #[error("Max retries {0} exceeded")]
    MaxRetriesExceeded(u8),

    /// General request error, retry might help
    #[error("Could not create request: {0}")]
    Request(String),

    /// Wrong network address
    #[error("Network address: {0}")]
    WrongNetworkAddress(Network),

    /// Server version is unexpected or incompatible
    #[error(transparent)]
    UnexpectedServerVersion(#[from] UnexpectedServerVersionError),

    /// Could not sign raw transaction
    #[error(transparent)]
    Sign(#[from] SignRawTransactionWithWalletError),

    /// Could not get a [`Xpriv`](bitcoin::bip32::Xpriv) from the wallet
    #[error("Could not get xpriv from wallet")]
    Xpriv,

    /// Unknown error, unlikely to be recoverable
    #[error("{0}")]
    Other(String),
}

impl ClientError {
    /// Returns `true` when the RPC server reports an invalid address, key, or missing
    /// transaction/block identifier (`RPC_INVALID_ADDRESS_OR_KEY`, code `-5`).
    pub fn is_tx_not_found(&self) -> bool {
        matches!(self, Self::Server(-5, _))
    }

    /// Returns `true` when the RPC server reports an invalid address, key, or missing
    /// transaction/block identifier (`RPC_INVALID_ADDRESS_OR_KEY`, code `-5`).
    pub fn is_block_not_found(&self) -> bool {
        matches!(self, Self::Server(-5, _))
    }

    /// Returns `true` when the RPC server reports a general transaction or block
    /// submission verification error (`RPC_VERIFY_ERROR`, code `-25`).
    pub fn is_rpc_verify_error(&self) -> bool {
        matches!(self, Self::Server(RPC_VERIFY_ERROR, _))
    }

    /// Returns `true` when the RPC server reports a transaction or block rejected
    /// by network rules (`RPC_VERIFY_REJECTED`, code `-26`).
    pub fn is_rpc_verify_rejected(&self) -> bool {
        matches!(self, Self::Server(RPC_VERIFY_REJECTED, _))
    }

    /// Returns `true` when the RPC server reports a transaction already present in
    /// the UTXO set (`RPC_VERIFY_ALREADY_IN_UTXO_SET`, code `-27`).
    pub fn is_rpc_verify_already_in_utxo_set(&self) -> bool {
        matches!(self, Self::Server(RPC_VERIFY_ALREADY_IN_UTXO_SET, _))
    }

    /// Returns `true` when the RPC server reports missing or invalid transaction
    /// inputs (`RPC_VERIFY_ERROR`, code `-25`).
    #[deprecated(
        since = "0.10.4",
        note = "use is_rpc_verify_error() to detect RPC_VERIFY_ERROR (-25)"
    )]
    pub fn is_missing_or_invalid_input(&self) -> bool {
        self.is_rpc_verify_error()
    }
}

impl From<BitreqError> for ClientError {
    fn from(value: BitreqError) -> Self {
        match value {
            // Connection errors
            BitreqError::AddressNotFound
            | BitreqError::IoError(_)
            | BitreqError::RustlsCreateConnection(_) => ClientError::Connection(value.to_string()),

            // Redirect errors
            BitreqError::RedirectLocationMissing
            | BitreqError::InfiniteRedirectionLoop
            | BitreqError::TooManyRedirections => ClientError::HttpRedirect(value.to_string()),

            // Size/parsing errors
            BitreqError::HeadersOverflow
            | BitreqError::StatusLineOverflow
            | BitreqError::BodyOverflow
            | BitreqError::MalformedChunkLength
            | BitreqError::MalformedChunkEnd
            | BitreqError::MalformedContentLength
            | BitreqError::InvalidUtf8InResponse
            | BitreqError::InvalidUtf8InBody(_) => {
                ClientError::MalformedResponse(value.to_string())
            }

            // Other errors
            _ => ClientError::Other(value.to_string()),
        }
    }
}

impl From<SerdeJsonError> for ClientError {
    fn from(value: SerdeJsonError) -> Self {
        Self::Parse(format!("Could not parse {value}"))
    }
}

/// `bitcoind` RPC server error.
#[derive(Error, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BitcoinRpcError {
    pub code: i32,
    pub message: String,
}

impl fmt::Display for BitcoinRpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RPC error {}: {}", self.code, self.message)
    }
}

impl From<BitcoinRpcError> for ClientError {
    fn from(value: BitcoinRpcError) -> Self {
        Self::Server(value.code, value.message)
    }
}

/// Error returned when signing a raw transaction with a wallet fails.
#[derive(Error, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignRawTransactionWithWalletError {
    /// The transaction ID.
    txid: String,
    /// The index of the input.
    vout: u32,
    /// The script signature.
    #[serde(rename = "scriptSig")]
    script_sig: String,
    /// The sequence number.
    sequence: u32,
    /// The error message.
    error: String,
}

impl fmt::Display for SignRawTransactionWithWalletError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "error signing raw transaction with wallet: {}",
            self.error
        )
    }
}

/// Error returned when RPC client expects a different version than bitcoind reports.
#[derive(Error, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnexpectedServerVersionError {
    /// Version from server.
    pub got: usize,
    /// Expected server version.
    pub expected: Vec<usize>,
}

impl fmt::Display for UnexpectedServerVersionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut expected = String::new();
        for version in &self.expected {
            let v = format!(" {version} ");
            expected.push_str(&v);
        }
        write!(
            f,
            "unexpected bitcoind version, got: {} expected one of: {}",
            self.got, expected
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(deprecated)]

    use super::ClientError;

    #[test]
    fn classifies_rpc_verify_error() {
        let error = ClientError::Server(-25, "Input not found or already spent".to_string());

        assert!(error.is_rpc_verify_error());
        assert!(error.is_missing_or_invalid_input());
        assert!(!error.is_rpc_verify_rejected());
        assert!(!error.is_rpc_verify_already_in_utxo_set());
    }

    #[test]
    fn classifies_rpc_verify_rejected() {
        let error = ClientError::Server(-26, "txn-already-in-mempool".to_string());

        assert!(error.is_rpc_verify_rejected());
        assert!(!error.is_missing_or_invalid_input());
        assert!(!error.is_rpc_verify_error());
        assert!(!error.is_rpc_verify_already_in_utxo_set());
    }

    #[test]
    fn classifies_rpc_verify_already_in_utxo_set() {
        let error = ClientError::Server(-27, "transaction already in block chain".to_string());

        assert!(error.is_rpc_verify_already_in_utxo_set());
        assert!(!error.is_rpc_verify_error());
        assert!(!error.is_rpc_verify_rejected());
        assert!(!error.is_missing_or_invalid_input());
    }

    #[test]
    fn non_server_errors_do_not_match_rpc_code_helpers() {
        let error = ClientError::Timeout;

        assert!(!error.is_rpc_verify_error());
        assert!(!error.is_rpc_verify_rejected());
        assert!(!error.is_rpc_verify_already_in_utxo_set());
        assert!(!error.is_missing_or_invalid_input());
    }
}
