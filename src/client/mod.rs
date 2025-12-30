use std::{
    fmt,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::error::{BitcoinRpcError, ClientError};
use base64::{engine::general_purpose, Engine};
use reqwest::{
    header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE},
    Client as ReqwestClient,
};
use serde::{de, Deserialize, Serialize};
use serde_json::{json, value::Value};
use tokio::time::sleep;
use tracing::*;

#[cfg(feature = "29_0")]
pub mod v29;

/// This is an alias for the result type returned by the [`Client`].
pub type ClientResult<T> = Result<T, ClientError>;

/// The maximum number of retries for a request.
const DEFAULT_MAX_RETRIES: u8 = 3;

/// The maximum number of retries for a request.
const DEFAULT_RETRY_INTERVAL_MS: u64 = 1_000;

/// The timeout for a request in seconds.
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;

/// Custom implementation to convert a value to a `Value` type.
pub fn to_value<T>(value: T) -> ClientResult<Value>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .map_err(|e| ClientError::Param(format!("Error creating value: {e}")))
}

/// The different authentication methods for the client.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum Auth {
    UserPass(String, String),
    CookieFile(PathBuf),
}

impl Auth {
    pub(crate) fn get_user_pass(self) -> ClientResult<(Option<String>, Option<String>)> {
        match self {
            Auth::UserPass(u, p) => Ok((Some(u), Some(p))),
            Auth::CookieFile(path) => {
                let line = BufReader::new(
                    File::open(path).map_err(|e| ClientError::Other(e.to_string()))?,
                )
                .lines()
                .next()
                .ok_or(ClientError::Other("Invalid cookie file".to_string()))?
                .map_err(|e| ClientError::Other(e.to_string()))?;
                let colon = line
                    .find(':')
                    .ok_or(ClientError::Other("Invalid cookie file".to_string()))?;
                Ok((Some(line[..colon].into()), Some(line[colon + 1..].into())))
            }
        }
    }
}

/// An `async` client for interacting with a `bitcoind` instance.
#[derive(Debug, Clone)]
pub struct Client {
    /// The URL of the `bitcoind` instance.
    url: String,

    /// The underlying `async` HTTP client.
    client: ReqwestClient,

    /// The ID of the current request.
    ///
    /// # Implementation Details
    ///
    /// Using an [`Arc`] so that [`Client`] is [`Clone`].
    id: Arc<AtomicUsize>,

    /// The maximum number of retries for a request.
    max_retries: u8,

    /// Interval between retries for a request in ms.
    retry_interval: u64,
}

/// Response returned by the `bitcoind` RPC server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Response<R> {
    pub result: Option<R>,
    pub error: Option<BitcoinRpcError>,
    pub id: u64,
}

impl Client {
    /// Creates a new [`Client`] with the given URL, username, and password.
    pub fn new(
        url: String,
        auth: Auth,
        max_retries: Option<u8>,
        retry_interval: Option<u64>,
        timeout: Option<u64>,
    ) -> ClientResult<Self> {
        let (username_opt, password_opt) = auth.get_user_pass()?;
        let (Some(username), Some(password)) = (
            username_opt.filter(|u| !u.is_empty()),
            password_opt.filter(|p| !p.is_empty()),
        ) else {
            return Err(ClientError::MissingUserPassword);
        };

        let user_pw = general_purpose::STANDARD.encode(format!("{username}:{password}"));
        let authorization = format!("Basic {user_pw}")
            .parse()
            .map_err(|_| ClientError::Other("Error parsing header".to_string()))?;

        let content_type = "application/json"
            .parse()
            .map_err(|_| ClientError::Other("Error parsing header".to_string()))?;
        let headers =
            HeaderMap::from_iter([(AUTHORIZATION, authorization), (CONTENT_TYPE, content_type)]);

        let client = ReqwestClient::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(
                timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            ))
            .build()
            .map_err(|e| ClientError::Other(format!("Could not create client: {e}")))?;

        let id = Arc::new(AtomicUsize::new(0));

        let max_retries = max_retries.unwrap_or(DEFAULT_MAX_RETRIES);
        let retry_interval = retry_interval.unwrap_or(DEFAULT_RETRY_INTERVAL_MS);

        trace!(url = %url, "Created bitcoin client");

        Ok(Self {
            url,
            client,
            id,
            max_retries,
            retry_interval,
        })
    }

    fn next_id(&self) -> usize {
        self.id.fetch_add(1, Ordering::AcqRel)
    }

    async fn call<T: de::DeserializeOwned + fmt::Debug>(
        &self,
        method: &str,
        params: &[Value],
    ) -> ClientResult<T> {
        let mut retries = 0;
        loop {
            trace!(%method, ?params, %retries, "Calling bitcoin client");

            let id = self.next_id();

            let response = self
                .client
                .post(&self.url)
                .json(&json!({
                    "jsonrpc": "1.0",
                    "id": id,
                    "method": method,
                    "params": params
                }))
                .send()
                .await;
            match response {
                Ok(resp) => {
                    // Check HTTP status code first before parsing body
                    let resp = match resp.error_for_status() {
                        Err(e) if e.is_status() => {
                            if let Some(status) = e.status() {
                                let reason =
                                    status.canonical_reason().unwrap_or("Unknown").to_string();
                                return Err(ClientError::Status(status.as_u16(), reason));
                            } else {
                                return Err(ClientError::Other(e.to_string()));
                            }
                        }
                        Err(e) => {
                            return Err(ClientError::Other(e.to_string()));
                        }
                        Ok(resp) => resp,
                    };

                    let raw_response = resp
                        .text()
                        .await
                        .map_err(|e| ClientError::Parse(e.to_string()))?;
                    trace!(%raw_response, "Raw response received");
                    let data: Response<T> = serde_json::from_str(&raw_response)
                        .map_err(|e| ClientError::Parse(e.to_string()))?;
                    if let Some(err) = data.error {
                        return Err(ClientError::Server(err.code, err.message));
                    }
                    return data
                        .result
                        .ok_or_else(|| ClientError::Other("Empty data received".to_string()));
                }
                Err(err) => {
                    warn!(err = %err, "Error calling bitcoin client");

                    if err.is_body() {
                        // Body error is unrecoverable
                        return Err(ClientError::Body(err.to_string()));
                    } else if err.is_status() {
                        // Status error is unrecoverable
                        let e = match err.status() {
                            Some(code) => ClientError::Status(code.as_u16(), err.to_string()),
                            _ => ClientError::Other(err.to_string()),
                        };
                        return Err(e);
                    } else if err.is_decode() {
                        // Error decoding response, might be recoverable
                        let e = ClientError::MalformedResponse(err.to_string());
                        warn!(%e, "decoding error, retrying...");
                    } else if err.is_connect() {
                        // Connection error, might be recoverable
                        let e = ClientError::Connection(err.to_string());
                        warn!(%e, "connection error, retrying...");
                    } else if err.is_timeout() {
                        // Timeout error, might be recoverable
                        let e = ClientError::Timeout;
                        warn!(%e, "timeout error, retrying...");
                    } else if err.is_request() {
                        // General request error, might be recoverable
                        let e = ClientError::Request(err.to_string());
                        warn!(%e, "request error, retrying...");
                    } else if err.is_builder() {
                        // Request builder error is unrecoverable
                        return Err(ClientError::ReqBuilder(err.to_string()));
                    } else if err.is_redirect() {
                        // Redirect error is unrecoverable
                        return Err(ClientError::HttpRedirect(err.to_string()));
                    } else {
                        // Unknown error is unrecoverable
                        return Err(ClientError::Other("Unknown error".to_string()));
                    }
                }
            }
            retries += 1;
            if retries >= self.max_retries {
                return Err(ClientError::MaxRetriesExceeded(self.max_retries));
            }
            sleep(Duration::from_millis(self.retry_interval)).await;
        }
    }

    #[cfg(feature = "raw_rpc")]
    /// Low-level RPC call wrapper; sends raw params and returns the deserialized result.
    pub async fn call_raw<R: de::DeserializeOwned + fmt::Debug>(
        &self,
        method: &str,
        params: &[serde_json::Value],
    ) -> ClientResult<R> {
        self.call::<R>(method, params).await
    }
}
