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
use bitcoin::{Amount, FeeRate};
use bitreq::{post, Client as BitreqClient, Error as BitreqError};
use serde::{de, Deserialize, Serialize};
use serde_json::{json, value::Value};
use tokio::time::sleep;
use tracing::*;

#[cfg(all(feature = "29_0", feature = "30_2"))]
compile_error!(
    "Bitcoin Core version features are mutually exclusive; select only one of `29_0` or `30_2`."
);

#[cfg(all(feature = "29_0", not(feature = "30_2")))]
pub mod v29;

#[cfg(all(feature = "30_2", not(feature = "29_0")))]
pub mod v30;

/// This is an alias for the result type returned by the [`Client`].
pub type ClientResult<T> = Result<T, ClientError>;

/// The maximum number of retries for a request.
const DEFAULT_MAX_RETRIES: u16 = 3;

/// The maximum number of retries for a request.
const DEFAULT_RETRY_INTERVAL_MS: u64 = 1_000;

/// The timeout for a request in seconds.
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;

/// The default capacity for the HTTP client connection pool.
const DEFAULT_HTTP_CLIENT_CAPACITY: usize = 10;

/// Custom implementation to convert a value to a `Value` type.
pub fn to_value<T>(value: T) -> ClientResult<Value>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .map_err(|e| ClientError::Param(format!("Error creating value: {e}")))
}

pub(crate) fn push_broadcast_options(
    params: &mut Vec<Value>,
    max_fee_rate: Option<FeeRate>,
    max_burn_amount: Option<Amount>,
) -> ClientResult<()> {
    if max_fee_rate.is_none() && max_burn_amount.is_none() {
        return Ok(());
    }

    match max_fee_rate {
        Some(max_fee_rate) => params.push(to_value(max_fee_rate_btc_per_kvb(max_fee_rate))?),
        None => params.push(Value::Null),
    }

    if let Some(max_burn_amount) = max_burn_amount {
        params.push(to_value(max_burn_amount.to_btc())?);
    }

    Ok(())
}

fn max_fee_rate_btc_per_kvb(max_fee_rate: FeeRate) -> f64 {
    max_fee_rate.to_sat_per_kwu() as f64 / 25_000_000.0
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
#[derive(Clone)]
pub struct Client {
    /// The URL of the `bitcoind` instance.
    url: String,

    /// The authorization header value for Basic auth.
    authorization: String,

    /// The timeout for requests in seconds.
    timeout: u64,

    /// The ID of the current request.
    ///
    /// # Implementation Details
    ///
    /// Using an [`Arc`] so that [`Client`] is [`Clone`].
    id: Arc<AtomicUsize>,

    /// The maximum number of retries for a request.
    max_retries: u16,

    /// Interval between retries for a request in ms.
    retry_interval: u64,

    /// The HTTP client for making requests.
    ///
    /// This is used to reuse TCP connections across requests.
    http_client: BitreqClient,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("url", &self.url)
            .field("timeout", &self.timeout)
            .field("id", &self.id)
            .field("max_retries", &self.max_retries)
            .field("retry_interval", &self.retry_interval)
            .finish_non_exhaustive()
    }
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
        max_retries: Option<u16>,
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
        let authorization = format!("Basic {user_pw}");

        let id = Arc::new(AtomicUsize::new(0));

        let max_retries = max_retries.unwrap_or(DEFAULT_MAX_RETRIES);
        let retry_interval = retry_interval.unwrap_or(DEFAULT_RETRY_INTERVAL_MS);
        let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS);

        let http_client = BitreqClient::new(DEFAULT_HTTP_CLIENT_CAPACITY);

        trace!(url = %url, "Created bitcoin client");

        Ok(Self {
            url,
            authorization,
            timeout,
            id,
            max_retries,
            retry_interval,
            http_client,
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
            debug!(%method, ?params, %retries, "Calling bitcoin client");

            let id = self.next_id();

            let body = serde_json::to_vec(&json!({
                "jsonrpc": "1.0",
                "id": id,
                "method": method,
                "params": params
            }))
            .map_err(|e| ClientError::Param(format!("Error serializing request: {e}")))?;

            let request = post(&self.url)
                .with_header("Authorization", &self.authorization)
                .with_header("Content-Type", "application/json")
                .with_body(body)
                .with_timeout(self.timeout);

            let response = self.http_client.send_async(request).await;

            match response {
                Ok(resp) => {
                    let status_code = resp.status_code;
                    let raw_response = resp
                        .as_str()
                        .map_err(|e| ClientError::Parse(e.to_string()))?;

                    if !(200..300).contains(&status_code) {
                        if let Ok(data) = serde_json::from_str::<Response<Value>>(raw_response) {
                            if let Some(err) = data.error {
                                return Err(ClientError::Server(err.code, err.message));
                            }
                        }

                        return Err(ClientError::Status(
                            status_code as u16,
                            format!("{} | body: {raw_response}", resp.reason_phrase),
                        ));
                    }

                    trace!(%raw_response, "Raw response received");
                    let data: Response<T> = serde_json::from_str(raw_response)
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

                    // Classify bitreq errors for retry logic
                    let should_retry = Self::is_error_recoverable(&err);
                    if !should_retry {
                        return Err(err.into());
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

    /// Returns `true` if the error is potentially recoverable and should be retried.
    fn is_error_recoverable(err: &BitreqError) -> bool {
        match err {
            // Connection/network errors - might be recoverable
            BitreqError::AddressNotFound
            | BitreqError::IoError(_)
            | BitreqError::RustlsCreateConnection(_) => {
                warn!(err = %err, "connection error, retrying...");
                true
            }

            // Redirect errors - not retryable
            BitreqError::RedirectLocationMissing => false,
            BitreqError::InfiniteRedirectionLoop => false,
            BitreqError::TooManyRedirections => false,

            // Size limit errors - not retryable
            BitreqError::HeadersOverflow => false,
            BitreqError::StatusLineOverflow => false,
            BitreqError::BodyOverflow => false,

            // Protocol/parsing errors - might be recoverable
            BitreqError::MalformedChunkLength
            | BitreqError::MalformedChunkEnd
            | BitreqError::MalformedContentLength
            | BitreqError::InvalidUtf8InResponse => {
                warn!(err = %err, "malformed response, retrying...");
                true
            }

            // UTF-8 in body - not retryable
            BitreqError::InvalidUtf8InBody(_) => false,

            // HTTPS not enabled - not retryable
            BitreqError::HttpsFeatureNotEnabled => false,

            // Other errors - not retryable
            BitreqError::Other(_) => false,

            // Non-exhaustive match fallback
            _ => false,
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::oneshot,
        time::{sleep, timeout},
    };

    use super::*;

    #[test]
    fn push_broadcast_options_omits_empty_options() {
        let mut params = vec![json!("rawtx")];

        push_broadcast_options(&mut params, None, None).unwrap();

        assert_eq!(params, vec![json!("rawtx")]);
    }

    #[test]
    fn push_broadcast_options_adds_max_fee_rate_only() {
        let mut params = vec![json!("rawtx")];

        push_broadcast_options(
            &mut params,
            Some(FeeRate::from_sat_per_kwu(25_000_000)),
            None,
        )
        .unwrap();

        assert_eq!(params, vec![json!("rawtx"), json!(1.0)]);
    }

    #[test]
    fn push_broadcast_options_adds_null_placeholder_for_max_burn_amount_only() {
        let mut params = vec![json!("rawtx")];

        push_broadcast_options(&mut params, None, Some(Amount::from_sat(50_000))).unwrap();

        assert_eq!(params, vec![json!("rawtx"), Value::Null, json!(0.0005)]);
    }

    #[test]
    fn push_broadcast_options_adds_max_fee_rate_and_max_burn_amount() {
        let mut params = vec![json!("rawtx")];

        push_broadcast_options(
            &mut params,
            Some(FeeRate::from_sat_per_kwu(12_500_000)),
            Some(Amount::from_sat(25_000)),
        )
        .unwrap();

        assert_eq!(params, vec![json!("rawtx"), json!(0.5), json!(0.00025)]);
    }

    async fn read_http_request(stream: &mut TcpStream) {
        let mut buf = vec![0u8; 4096];
        let mut total = Vec::new();
        loop {
            let n = stream.read(&mut buf).await.expect("read request");
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buf[..n]);
            let Some(hdr_end) = total.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let headers = std::str::from_utf8(&total[..hdr_end]).unwrap_or("");
            let cl: usize = headers
                .lines()
                .find_map(|l| {
                    let mut parts = l.splitn(2, ':');
                    let k = parts.next()?.trim();
                    if k.eq_ignore_ascii_case("Content-Length") {
                        parts.next()?.trim().parse().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            if total.len() >= hdr_end + 4 + cl {
                break;
            }
        }
    }

    async fn write_json_response(stream: &mut TcpStream, body: &str) {
        let response = format!(
            concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: application/json\r\n",
                "Connection: keep-alive\r\n",
                "Content-Length: {}\r\n\r\n{}"
            ),
            body.len(),
            body,
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        stream.flush().await.expect("flush response");
    }

    /// Regression test for issue #101: a pooled keep-alive socket that is later
    /// closed server-side must not permanently poison future RPC calls.
    #[tokio::test]
    async fn retry_recovers_from_dead_pooled_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let (ready_tx, ready_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut first_stream, _) = listener.accept().await.expect("accept 1");
            read_http_request(&mut first_stream).await;
            write_json_response(
                &mut first_stream,
                r#"{"result":"first","error":null,"id":0}"#,
            )
            .await;

            // Keep the socket alive long enough for the client to cache it, then
            // close it server-side to mimic bitcoind's rpcservertimeout behavior.
            sleep(Duration::from_millis(100)).await;
            drop(first_stream);
            let _ = ready_tx.send(());

            let (mut second_stream, _) = listener.accept().await.expect("accept 2");
            read_http_request(&mut second_stream).await;
            write_json_response(
                &mut second_stream,
                r#"{"result":"second","error":null,"id":1}"#,
            )
            .await;
        });

        let url = format!("http://{}", addr);
        let client = Client::new(
            url,
            Auth::UserPass("user".into(), "pass".into()),
            Some(3),
            Some(10),
            Some(5),
        )
        .expect("client");

        let first: String = client.call("ping", &[]).await.expect("first call");
        assert_eq!(first, "first");

        ready_rx.await.expect("ready signal");

        let second: String = timeout(Duration::from_secs(5), client.call("ping", &[]))
            .await
            .expect("call did not time out")
            .expect("second call");
        assert_eq!(second, "second");

        server.await.expect("server task");
    }
}
