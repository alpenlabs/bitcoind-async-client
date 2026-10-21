use std::{
    fmt,
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, PoisonError, RwLock,
    },
    time::Duration,
};

use crate::error::{BitcoinRpcError, ClientError};
use base64::{engine::general_purpose, Engine};
use bitreq::{post, Client as BitreqClient, Error as BitreqError};
use serde::{de, Deserialize, Serialize};
use serde_json::{json, value::Value};
use tokio::time::sleep;
use tracing::*;

#[cfg(any(
    all(feature = "29_0", feature = "30_2"),
    all(feature = "29_0", feature = "31_1"),
    all(feature = "30_2", feature = "31_1")
))]
compile_error!(
    "Bitcoin Core version features are mutually exclusive; select only one of `29_0`, `30_2`, or `31_1`."
);

#[cfg(all(feature = "29_0", not(any(feature = "30_2", feature = "31_1"))))]
pub mod v29;

#[cfg(all(feature = "30_2", not(any(feature = "29_0", feature = "31_1"))))]
pub mod v30;

#[cfg(all(feature = "31_1", not(any(feature = "29_0", feature = "30_2"))))]
pub mod v31;

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

/// The maximum size of a cookie file in bytes.
const MAX_COOKIE_FILE_BYTES: u64 = 1024;

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
    /// Authenticate with a username and password.
    UserPass(String, String),
    /// Read credentials from bitcoind's cookie file. After a 401, the client reads
    /// the file again and retries the request once if the cookie changed, so it
    /// keeps working across bitcoind restarts.
    ///
    /// The file must not be writable by untrusted users, since the client reads
    /// it again on the request path. The retry relies on the server rejecting
    /// bad credentials before running the request, as bitcoind does. A proxy
    /// that returns 401 after forwarding a request could see that request twice.
    CookieFile(PathBuf),
}

impl Auth {
    pub(crate) fn get_user_pass(self) -> ClientResult<(Option<String>, Option<String>)> {
        match self {
            Auth::UserPass(u, p) => Ok((Some(u), Some(p))),
            Auth::CookieFile(path) => {
                let metadata = std::fs::metadata(&path)
                    .map_err(|e| ClientError::Other(format!("Could not read cookie file: {e}")))?;
                if !metadata.is_file() {
                    return Err(ClientError::Other(
                        "Cookie file is not a regular file".into(),
                    ));
                }
                let file = File::open(path)
                    .map_err(|e| ClientError::Other(format!("Could not read cookie file: {e}")))?;
                let mut bytes = Vec::new();
                file.take(MAX_COOKIE_FILE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| ClientError::Other(format!("Could not read cookie file: {e}")))?;
                if bytes.len() as u64 > MAX_COOKIE_FILE_BYTES {
                    return Err(ClientError::Other("Cookie file is too large".into()));
                }
                let contents = std::str::from_utf8(&bytes)
                    .map_err(|_| ClientError::Other("Invalid cookie file".into()))?;
                let line = contents
                    .lines()
                    .next()
                    .ok_or(ClientError::Other("Invalid cookie file".into()))?;
                let colon = line
                    .find(':')
                    .ok_or(ClientError::Other("Invalid cookie file".to_string()))?;
                Ok((Some(line[..colon].into()), Some(line[colon + 1..].into())))
            }
        }
    }
}

/// Basic auth credentials, shared by all clones of a [`Client`].
struct Credentials {
    /// Cookie file to re-read after a 401, for [`Auth::CookieFile`].
    cookie_file: Option<PathBuf>,
    /// Current `Authorization` header value.
    header: RwLock<Arc<str>>,
}

impl Credentials {
    /// Returns an owned snapshot of the current authorization header.
    fn header(&self) -> Arc<str> {
        self.header
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Re-reads the cookie on a blocking thread after a request sent with `stale`
    /// got a 401. Returns true when a retry may succeed.
    async fn refresh(&self, stale: &str) -> bool {
        let Some(path) = &self.cookie_file else {
            return false;
        };
        // Another clone or task may have reloaded the cookie already.
        if &*self.header() != stale {
            return true;
        }
        let path = path.clone();
        let result =
            tokio::task::spawn_blocking(move || authorization_header(Auth::CookieFile(path))).await;
        match result {
            Ok(Ok(header)) if &*header != stale => self.publish(stale, header),
            Ok(Ok(_)) => false,
            Ok(Err(err)) => {
                warn!(%err, "Error refreshing RPC credentials");
                false
            }
            Err(err) => {
                warn!(%err, "Error refreshing RPC credentials");
                false
            }
        }
    }

    /// Replaces the header only if it still holds `stale`, so a slow refresh
    /// cannot roll back a newer cookie. Returns true because a retry with the
    /// current header may succeed.
    fn publish(&self, stale: &str, header: Arc<str>) -> bool {
        let mut current = self.header.write().unwrap_or_else(PoisonError::into_inner);
        if &**current == stale {
            *current = header;
        }
        true
    }
}

/// Builds a Basic authorization header from nonempty credentials.
fn authorization_header(auth: Auth) -> ClientResult<Arc<str>> {
    let (username_opt, password_opt) = auth.get_user_pass()?;
    let (Some(username), Some(password)) = (
        username_opt.filter(|u| !u.is_empty()),
        password_opt.filter(|p| !p.is_empty()),
    ) else {
        return Err(ClientError::MissingUserPassword);
    };

    let user_pw = general_purpose::STANDARD.encode(format!("{username}:{password}"));
    Ok(format!("Basic {user_pw}").into())
}

/// An `async` client for interacting with a `bitcoind` instance.
#[derive(Clone)]
pub struct Client {
    /// The URL of the `bitcoind` instance.
    url: String,

    /// Basic auth credentials shared by all clones.
    credentials: Arc<Credentials>,

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
        let cookie_file = match &auth {
            Auth::CookieFile(path) => Some(path.clone()),
            Auth::UserPass(..) => None,
        };
        let credentials = Arc::new(Credentials {
            cookie_file,
            header: RwLock::new(authorization_header(auth)?),
        });

        let id = Arc::new(AtomicUsize::new(0));

        let max_retries = max_retries.unwrap_or(DEFAULT_MAX_RETRIES);
        let retry_interval = retry_interval.unwrap_or(DEFAULT_RETRY_INTERVAL_MS);
        let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS);

        let http_client = BitreqClient::new(DEFAULT_HTTP_CLIENT_CAPACITY);

        trace!("Created bitcoin client");

        Ok(Self {
            url,
            credentials,
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
        let mut auth_refreshed = false;
        loop {
            let authorization = self.credentials.header();
            // RPC parameters and responses can contain private descriptors and keys.
            debug!(%method, %retries, "Calling bitcoin client");

            let id = self.next_id();

            let body = serde_json::to_vec(&json!({
                "jsonrpc": "1.0",
                "id": id,
                "method": method,
                "params": params
            }))
            .map_err(|e| ClientError::Param(format!("Error serializing request: {e}")))?;

            // bitcoind never redirects. Following a redirect would resend the
            // credentials and the request body to whatever host it names.
            let request = post(&self.url)
                .with_header("Authorization", &*authorization)
                .with_header("Content-Type", "application/json")
                .with_body(body)
                .with_timeout(self.timeout)
                .with_max_redirects(0);

            let response = self.http_client.send_async(request).await;

            match response {
                Ok(resp) => {
                    let status_code = resp.status_code;
                    if status_code == 401
                        && !auth_refreshed
                        && self.credentials.refresh(&authorization).await
                    {
                        auth_refreshed = true;
                        debug!(%method, "Retrying RPC with refreshed credentials");
                        continue;
                    }
                    if status_code == 401 {
                        return Err(ClientError::Status(401, resp.reason_phrase.clone()));
                    }
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

                    trace!(%method, %status_code, "Bitcoin RPC response received");
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
    use std::{
        io::{self, Write},
        sync::Mutex,
        time::Duration,
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::oneshot,
        time::{sleep, timeout},
    };
    use tracing::subscriber::NoSubscriber;
    use tracing_subscriber::util::SubscriberInitExt;

    use super::*;

    #[derive(Clone, Default)]
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    impl Write for LogBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Exercise the shared transport with private descriptor request/response data.
    /// A current-thread runtime keeps the scoped tracing subscriber on every poll.
    #[tokio::test(flavor = "current_thread")]
    async fn rpc_logs_omit_credentials_and_payloads() {
        // Avoid tracing's single-dispatcher optimization caching another test
        // thread's empty subscriber when it first uses a shared RPC callsite.
        let _other_dispatch = Dispatch::new(NoSubscriber::default());
        let logs = LogBuffer::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::TRACE)
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        let _guard = subscriber.set_default();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            for body in [
                r#"{"result":[{"success":true}],"error":null,"id":0}"#,
                r#"{"result":{"descriptors":[{"desc":"tr(response-private-key)"}]},"error":null,"id":1}"#,
                r#"{"result":null,"error":{"code":-5,"message":"error-private-key"},"id":2}"#,
            ] {
                read_http_request(&mut stream).await;
                write_json_response(&mut stream, body).await;
            }
        });

        let client = Client::new(
            format!("http://{address}/wallet/private-wallet?token=url-secret"),
            Auth::UserPass("rpc-user-secret".into(), "rpc-password-secret".into()),
            Some(1),
            None,
            Some(5),
        )
        .unwrap();
        debug!(?client, "Client configuration");

        let imported: Value = client
            .call(
                "importdescriptors",
                &[json!([{"desc": "tr(request-private-key)", "timestamp": "now"}])],
            )
            .await
            .unwrap();
        assert_eq!(imported, json!([{"success": true}]));
        let descriptors: Value = client
            .call("listdescriptors", &[json!(true)])
            .await
            .unwrap();
        assert_eq!(
            descriptors["descriptors"][0]["desc"],
            "tr(response-private-key)"
        );
        let error = client
            .call::<Value>("importdescriptors", &[json!("invalid-private-key")])
            .await
            .unwrap_err();
        assert!(
            matches!(error, ClientError::Server(-5, message) if message == "error-private-key")
        );
        server.await.unwrap();

        let output = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        // Ensure capture is active and useful metadata remains available.
        assert!(output.contains("Calling bitcoin client"));
        assert!(output.contains("importdescriptors"));
        assert!(output.contains("listdescriptors"));
        assert!(output.contains("status_code=200"));
        for secret in [
            "request-private-key",
            "response-private-key",
            "invalid-private-key",
            "error-private-key",
            "rpc-user-secret",
            "rpc-password-secret",
            "private-wallet",
            "url-secret",
            &client.credentials.header(),
        ] {
            assert!(!output.contains(secret), "sensitive value appeared in logs");
        }
    }

    async fn read_http_request(stream: &mut TcpStream) -> String {
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
                return headers.to_string();
            }
        }
        String::new()
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

    pub(super) fn temp_cookie_file(name: &str) -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "bitcoind-async-client-{name}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ))
    }

    async fn write_unauthorized_response(stream: &mut TcpStream) {
        stream
            .write_all(
                concat!(
                    "HTTP/1.1 401 Unauthorized\r\n",
                    "Connection: keep-alive\r\n",
                    "WWW-Authenticate: Basic realm=\"jsonrpc\"\r\n",
                    "Content-Length: 0\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("write response");
        stream.flush().await.expect("flush response");
    }

    async fn auth_server(
        succeed_after_401: bool,
    ) -> (
        String,
        oneshot::Sender<()>,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (stop, stopped) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            tokio::select! {
                _ = stopped => {},
                _ = async {
                    loop {
                        let (mut stream, _) = listener.accept().await.expect("accept");
                        loop {
                            let head = read_http_request(&mut stream).await;
                            if head.is_empty() {
                                break;
                            }
                            requests.push(head);
                            if succeed_after_401 && requests.len() > 1 {
                                write_json_response(
                                    &mut stream,
                                    r#"{"result":"ok","error":null,"id":0}"#,
                                ).await;
                            } else {
                                write_unauthorized_response(&mut stream).await;
                            }
                        }
                    }
                } => {},
            }
            requests
        });
        (format!("http://{addr}"), stop, server)
    }

    fn assert_authorization(head: &str, cookie: &str) {
        let header = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("Authorization")
                .then_some(value.trim())
        });
        let expected = format!("Basic {}", general_purpose::STANDARD.encode(cookie));
        assert_eq!(header, Some(expected.as_str()));
    }

    #[tokio::test]
    async fn cookie_refresh_retries_once_after_401() {
        let path = temp_cookie_file("refresh");
        std::fs::write(&path, "__cookie__:a").unwrap();
        let (url, stop, server) = auth_server(true).await;
        let client =
            Client::new(url, Auth::CookieFile(path.clone()), Some(0), None, Some(5)).unwrap();
        std::fs::write(&path, "__cookie__:b").unwrap();

        let result: String = client.call("ping", &[]).await.unwrap();
        assert_eq!(result, "ok");
        stop.send(()).unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_authorization(&requests[0], "__cookie__:a");
        assert_authorization(&requests[1], "__cookie__:b");
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn unchanged_cookie_returns_401_without_retry() {
        let path = temp_cookie_file("unchanged");
        std::fs::write(&path, "__cookie__:a").unwrap();
        let (url, stop, server) = auth_server(false).await;
        let client =
            Client::new(url, Auth::CookieFile(path.clone()), Some(0), None, Some(5)).unwrap();

        let error = client.call::<Value>("ping", &[]).await.unwrap_err();
        assert!(matches!(error, ClientError::Status(401, _)));
        stop.send(()).unwrap();
        assert_eq!(server.await.unwrap().len(), 1);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn user_pass_401_is_not_retried() {
        let (url, stop, server) = auth_server(false).await;
        let client = Client::new(
            url,
            Auth::UserPass("user".into(), "pass".into()),
            Some(0),
            None,
            Some(5),
        )
        .unwrap();

        let error = client.call::<Value>("ping", &[]).await.unwrap_err();
        assert!(matches!(error, ClientError::Status(401, _)));
        stop.send(()).unwrap();
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn missing_cookie_after_401_returns_401() {
        let path = temp_cookie_file("missing");
        std::fs::write(&path, "__cookie__:a").unwrap();
        let (url, stop, server) = auth_server(false).await;
        let client =
            Client::new(url, Auth::CookieFile(path.clone()), Some(0), None, Some(5)).unwrap();
        std::fs::remove_file(path).unwrap();

        let error = client.call::<Value>("ping", &[]).await.unwrap_err();
        assert!(matches!(error, ClientError::Status(401, _)));
        stop.send(()).unwrap();
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn clones_share_refreshed_cookie() {
        let path = temp_cookie_file("clones");
        std::fs::write(&path, "__cookie__:a").unwrap();
        let (url, stop, server) = auth_server(true).await;
        let client =
            Client::new(url, Auth::CookieFile(path.clone()), Some(0), None, Some(5)).unwrap();
        let clone = client.clone();
        std::fs::write(&path, "__cookie__:b").unwrap();

        let result: String = client.call("ping", &[]).await.unwrap();
        assert_eq!(result, "ok");
        let result: String = clone.call("ping", &[]).await.unwrap();
        assert_eq!(result, "ok");
        stop.send(()).unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_authorization(&requests[0], "__cookie__:a");
        assert_authorization(&requests[1], "__cookie__:b");
        assert_authorization(&requests[2], "__cookie__:b");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cookie_file_accepts_missing_newline() {
        let path = temp_cookie_file("no-newline");
        std::fs::write(&path, "user:pass").unwrap();
        let header = authorization_header(Auth::CookieFile(path.clone())).unwrap();
        assert_eq!(&*header, "Basic dXNlcjpwYXNz");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cookie_file_accepts_crlf() {
        let path = temp_cookie_file("crlf");
        std::fs::write(&path, "user:pass\r\n").unwrap();
        let header = authorization_header(Auth::CookieFile(path.clone())).unwrap();
        assert_eq!(&*header, "Basic dXNlcjpwYXNz");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cookie_file_rejects_oversized_line() {
        let path = temp_cookie_file("oversized");
        std::fs::write(&path, format!("user:{}", "p".repeat(2048 - 5))).unwrap();
        assert!(Client::new(
            "http://127.0.0.1".into(),
            Auth::CookieFile(path.clone()),
            None,
            None,
            None,
        )
        .is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cookie_file_rejects_directory() {
        let path = temp_cookie_file("directory");
        std::fs::create_dir(&path).unwrap();
        assert!(Client::new(
            "http://127.0.0.1".into(),
            Auth::CookieFile(path.clone()),
            None,
            None,
            None,
        )
        .is_err());
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn cookie_refresh_keeps_newer_header() {
        let credentials = Credentials {
            cookie_file: None,
            header: RwLock::new(Arc::from("newer")),
        };

        assert!(credentials.publish("stale", Arc::from("older")));
        assert_eq!(&*credentials.header(), "newer");
        assert!(credentials.publish("newer", Arc::from("latest")));
        assert_eq!(&*credentials.header(), "latest");
    }

    #[test]
    fn credentials_recover_from_poisoned_lock() {
        let credentials = Credentials {
            cookie_file: None,
            header: RwLock::new(Arc::from("stale")),
        };
        let result = std::panic::catch_unwind(|| {
            let _guard = credentials
                .header
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            panic!("poison credentials lock");
        });
        assert!(result.is_err());

        assert_eq!(&*credentials.header(), "stale");
        assert!(credentials.publish("stale", Arc::from("fresh")));
        assert_eq!(&*credentials.header(), "fresh");
    }

    #[tokio::test]
    async fn unauthorized_401_body_is_not_in_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            read_http_request(&mut stream).await;
            let body = "echoed-secret";
            let response = format!(
                concat!(
                    "HTTP/1.1 401 Unauthorized\r\n",
                    "WWW-Authenticate: Basic realm=\"jsonrpc\"\r\n",
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
        });
        let client = Client::new(
            format!("http://{addr}"),
            Auth::UserPass("user".into(), "pass".into()),
            Some(0),
            None,
            Some(5),
        )
        .unwrap();

        let error = client.call::<Value>("ping", &[]).await.unwrap_err();
        assert!(matches!(error, ClientError::Status(401, _)));
        assert!(!error.to_string().contains("echoed-secret"));
        assert!(!format!("{error:?}").contains("echoed-secret"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn changed_cookie_still_returns_401_after_one_retry() {
        let path = temp_cookie_file("retry-limit");
        std::fs::write(&path, "__cookie__:a").unwrap();
        let (url, stop, server) = auth_server(false).await;
        let client =
            Client::new(url, Auth::CookieFile(path.clone()), Some(0), None, Some(5)).unwrap();
        std::fs::write(&path, "__cookie__:b").unwrap();

        let error = client.call::<Value>("ping", &[]).await.unwrap_err();
        assert!(matches!(error, ClientError::Status(401, _)));
        stop.send(()).unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_authorization(&requests[0], "__cookie__:a");
        assert_authorization(&requests[1], "__cookie__:b");
        std::fs::remove_file(path).unwrap();
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

    /// A redirect must not forward the `Authorization` header or the RPC body
    /// to another host.
    #[tokio::test]
    async fn rpc_does_not_follow_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let target = TcpListener::bind("127.0.0.1:0").await.expect("bind target");
        let target_addr = target.local_addr().expect("target addr");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            read_http_request(&mut stream).await;
            // 307 keeps the method and body, the worst case for a leak.
            let response = format!(
                concat!(
                    "HTTP/1.1 307 Temporary Redirect\r\n",
                    "Location: http://{}/\r\n",
                    "Content-Length: 0\r\n\r\n"
                ),
                target_addr,
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            stream.flush().await.expect("flush response");
        });

        let client = Client::new(
            format!("http://{addr}"),
            Auth::UserPass("user".into(), "pass".into()),
            Some(1),
            None,
            Some(5),
        )
        .expect("client");

        let error = client.call::<Value>("ping", &[]).await.unwrap_err();
        assert!(
            matches!(error, ClientError::HttpRedirect(_)),
            "unexpected error: {error:?}"
        );
        server.await.expect("server task");

        // Any connection to the redirect target would already be queued.
        assert!(
            timeout(Duration::from_millis(100), target.accept())
                .await
                .is_err(),
            "client connected to the redirect target"
        );
    }
}
