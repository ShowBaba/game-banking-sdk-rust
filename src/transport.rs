use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{Error, Result};

/// The SDK version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A boxed future, as returned by [`HttpTransport::send`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The environment a game API key belongs to, read from its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Environment {
    /// `gbe_test_sk_…` keys.
    Sandbox,
    /// `gbe_live_sk_…` keys.
    Production,
}

impl Environment {
    /// Reads the environment from a game API key's prefix; `None` for other credentials.
    pub fn of(api_key: &str) -> Option<Environment> {
        if api_key.starts_with("gbe_test_sk_") {
            Some(Environment::Sandbox)
        } else if api_key.starts_with("gbe_live_sk_") {
            Some(Environment::Production)
        } else {
            None
        }
    }

    /// `"sandbox"` or `"production"`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Sandbox => "sandbox",
            Environment::Production => "production",
        }
    }
}

/// Passed to the response hook after every HTTP response, including retried ones.
#[derive(Debug, Clone)]
pub struct ResponseInfo {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub request_id: Option<String>,
    /// True when the engine returned a stored result for a repeated Idempotency-Key.
    pub replayed: bool,
    /// 0 for the first attempt.
    pub attempt: u32,
}

/// One HTTP request, as handed to an [`HttpTransport`].
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// The per-attempt timeout.
    pub timeout: Duration,
}

/// One HTTP response, as returned by an [`HttpTransport`].
#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The first value of a header, matched case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Why no response arrived.
#[derive(Debug, Clone)]
pub enum TransportError {
    /// No response within the timeout.
    Timeout,
    /// The connection failed or broke.
    Connection(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Timeout => f.write_str("timed out"),
            TransportError::Connection(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for TransportError {}

/// Sends HTTP requests. The default is [`ReqwestTransport`]; supply your own
/// with [`ClientBuilder::transport`], e.g. to route through a proxy or for tests.
pub trait HttpTransport: Send + Sync + 'static {
    fn send(
        &self,
        request: HttpRequest,
    ) -> BoxFuture<'_, std::result::Result<HttpResponse, TransportError>>;
}

/// The default transport, built on a `reqwest::Client`.
#[derive(Debug, Clone, Default)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(client: reqwest::Client) -> Self {
        ReqwestTransport { client }
    }
}

fn transport_error(e: reqwest::Error) -> TransportError {
    if e.is_timeout() {
        TransportError::Timeout
    } else {
        let mut msg = e.to_string();
        let mut src = std::error::Error::source(&e);
        while let Some(s) = src {
            msg = format!("{msg}: {s}");
            src = s.source();
        }
        TransportError::Connection(msg)
    }
}

impl HttpTransport for ReqwestTransport {
    fn send(
        &self,
        req: HttpRequest,
    ) -> BoxFuture<'_, std::result::Result<HttpResponse, TransportError>> {
        Box::pin(async move {
            let mut b = self
                .client
                .request(req.method, &req.url)
                .timeout(req.timeout);
            for (k, v) in req.headers {
                b = b.header(k, v);
            }
            if let Some(body) = req.body {
                b = b.body(body);
            }
            let res = b.send().await.map_err(transport_error)?;
            let status = res.status().as_u16();
            let headers = res
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_string(), v.to_string()))
                })
                .collect();
            let body = res.bytes().await.map_err(transport_error)?.to_vec();
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

/// Per-call settings. Set them on a resource before the call:
///
/// ```no_run
/// # async fn run(gb: gamebanking::GameBank, p: gamebanking::FundingParams) -> gamebanking::Result<()> {
/// gb.funding().idempotency_key("payroll-2040-06-01-user-42").create(&p).await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Default)]
pub struct RequestOptions {
    /// Your own stable key for a money-moving call. Retrying with the same key
    /// never moves money twice. Without one the SDK generates a key per call.
    pub idempotency_key: Option<String>,
    /// Overrides the client's retry count for this call.
    pub max_retries: Option<u32>,
}

pub(crate) type Hook = Arc<dyn Fn(&ResponseInfo) + Send + Sync>;
pub(crate) type Sleeper = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;

#[doc(hidden)]
pub struct Inner {
    api_key: String,
    base_url: String,
    timeout: Duration,
    max_retries: u32,
    transport: Arc<dyn HttpTransport>,
    on_response: Option<Hook>,
    headers: Vec<(String, String)>,
    pub(crate) sleep: Sleeper,
    pub(crate) environment: Option<Environment>,
}

// Never prints the credential.
impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("base_url", &self.base_url)
            .field("environment", &self.environment)
            .finish_non_exhaustive()
    }
}

impl Inner {
    /// A copy that sends extra headers on every request.
    pub(crate) fn with_headers(&self, extra: &[(&str, &str)]) -> Inner {
        let mut headers = self.headers.clone();
        headers.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        Inner {
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            timeout: self.timeout,
            max_retries: self.max_retries,
            transport: self.transport.clone(),
            on_response: self.on_response.clone(),
            headers,
            sleep: self.sleep.clone(),
            environment: self.environment,
        }
    }
}

/// Builds a [`GameBank`](crate::GameBank) or a [`GameBankManagement`](crate::GameBankManagement).
pub struct ClientBuilder<C> {
    api_key: Option<String>,
    base_url: Option<String>,
    timeout: Duration,
    max_retries: u32,
    transport: Option<Arc<dyn HttpTransport>>,
    on_response: Option<Hook>,
    _client: PhantomData<fn() -> C>,
}

// Never prints the credential.
impl<C> std::fmt::Debug for ClientBuilder<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .finish_non_exhaustive()
    }
}

/// Clients a [`ClientBuilder`] can build.
pub trait FromInner: Sized + sealed::Sealed {
    #[doc(hidden)]
    fn from_inner(inner: Arc<Inner>) -> Self;
}

pub(crate) mod sealed {
    pub trait Sealed {}
}

impl<C: FromInner> ClientBuilder<C> {
    pub(crate) fn new() -> Self {
        ClientBuilder {
            api_key: None,
            base_url: None,
            timeout: Duration::from_secs(30),
            max_retries: 2,
            transport: None,
            on_response: None,
            _client: PhantomData,
        }
    }

    /// The game API key or developer credential (default: `GBE_API_KEY`).
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Your engine's address (default: `GBE_BASE_URL`).
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// The per-attempt timeout (default 30 s).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// How many extra attempts a retryable failure gets (default 2).
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Uses your own `reqwest::Client`, e.g. with a proxy.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.transport = Some(Arc::new(ReqwestTransport::new(client)));
        self
    }

    /// Uses your own transport.
    pub fn transport(mut self, transport: impl HttpTransport) -> Self {
        self.transport = Some(Arc::new(transport));
        self
    }

    /// Called after every HTTP response with its status, request ID and
    /// whether the engine replayed an idempotent response.
    pub fn on_response(mut self, hook: impl Fn(&ResponseInfo) + Send + Sync + 'static) -> Self {
        self.on_response = Some(Arc::new(hook));
        self
    }

    /// Builds the client. A missing API key or base URL is read from
    /// `GBE_API_KEY` and `GBE_BASE_URL`.
    pub fn build(self) -> Result<C> {
        let api_key = self
            .api_key
            .or_else(|| std::env::var("GBE_API_KEY").ok())
            .unwrap_or_default();
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            return Err(Error::local(
                "missing_api_key",
                "pass api_key or set GBE_API_KEY",
            ));
        }
        let base_url = self
            .base_url
            .or_else(|| std::env::var("GBE_BASE_URL").ok())
            .unwrap_or_default();
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(Error::local(
                "missing_base_url",
                "pass base_url (your engine's address) or set GBE_BASE_URL",
            ));
        }
        let inner = Inner {
            environment: Environment::of(&api_key),
            api_key,
            base_url,
            timeout: self.timeout,
            max_retries: self.max_retries,
            transport: self
                .transport
                .unwrap_or_else(|| Arc::new(ReqwestTransport::default())),
            on_response: self.on_response,
            headers: Vec::new(),
            sleep: Arc::new(|d| Box::pin(tokio::time::sleep(d))),
        };
        Ok(C::from_inner(Arc::new(inner)))
    }
}

/// Percent-encodes a path segment or query value.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `api_path!("/v1/accounts/{}", id)` formats a path with escaped segments.
macro_rules! api_path {
    ($fmt:literal $(, $id:expr)* $(,)?) => {
        format!($fmt $(, $crate::transport::escape(AsRef::<str>::as_ref(&$id)))*)
    };
}
pub(crate) use api_path;

/// Query parameters; unset values are left out.
#[derive(Debug, Default, Clone)]
pub(crate) struct Query(Vec<(&'static str, String)>);

impl Query {
    pub(crate) fn new() -> Self {
        Query::default()
    }

    pub(crate) fn set(mut self, key: &'static str, value: impl ToString) -> Self {
        self.0.push((key, value.to_string()));
        self
    }

    pub(crate) fn opt<V: ToString>(self, key: &'static str, value: Option<V>) -> Self {
        match value {
            Some(v) => self.set(key, v),
            None => self,
        }
    }

    fn encode(&self) -> String {
        self.0
            .iter()
            .map(|(k, v)| format!("{k}={}", escape(v)))
            .collect::<Vec<_>>()
            .join("&")
    }
}

/// One operation on the wire.
pub(crate) struct Op {
    method: Method,
    path: String,
    query: Query,
    body: Option<Vec<u8>>,
    body_error: Option<serde_json::Error>,
    /// Money-moving: send an Idempotency-Key (generated when none is given).
    idempotent: bool,
    /// Safe to retry after a possible partial failure even without a key.
    retry_safe: bool,
}

impl Op {
    fn new(method: Method, path: String) -> Self {
        Op {
            method,
            path,
            query: Query::new(),
            body: None,
            body_error: None,
            idempotent: false,
            retry_safe: false,
        }
    }

    pub(crate) fn get(path: String) -> Self {
        Op::new(Method::GET, path)
    }

    pub(crate) fn post(path: String) -> Self {
        Op::new(Method::POST, path)
    }

    pub(crate) fn put(path: String) -> Self {
        Op::new(Method::PUT, path)
    }

    pub(crate) fn patch(path: String) -> Self {
        Op::new(Method::PATCH, path)
    }

    pub(crate) fn delete(path: String) -> Self {
        Op::new(Method::DELETE, path)
    }

    pub(crate) fn query(mut self, q: Query) -> Self {
        self.query = q;
        self
    }

    pub(crate) fn json(mut self, body: &impl Serialize) -> Self {
        match serde_json::to_vec(body) {
            Ok(b) => self.body = Some(b),
            Err(e) => self.body_error = Some(e),
        }
        self
    }

    pub(crate) fn idempotent(mut self) -> Self {
        self.idempotent = true;
        self
    }

    pub(crate) fn retry_safe(mut self) -> Self {
        self.retry_safe = true;
        self
    }
}

pub(crate) fn check_amount(v: i64, required: bool) -> Result<()> {
    if v < 0 || (required && v == 0) {
        return Err(Error::validation(
            "amount_minor must be a positive integer number of minor units",
        ));
    }
    Ok(())
}

impl Inner {
    /// Runs an operation and decodes its JSON response.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        op: Op,
        opts: &RequestOptions,
    ) -> Result<T> {
        let (res, rid) = self.execute(op, opts).await?;
        serde_json::from_slice(&res.body).map_err(|e| {
            let ct = res.header("content-type").unwrap_or("an unknown content type");
            Error {
                status: res.status,
                request_id: rid,
                ..Error::local(
                    "invalid_response",
                    format!(
                        "expected JSON from the engine but got {ct}; check that the base URL ({}) points at the engine, not a website",
                        self.base_url
                    ),
                )
            }
            .with_source(e)
        })
    }

    /// Runs an operation whose response has no body worth reading.
    pub(crate) async fn call_empty(&self, op: Op, opts: &RequestOptions) -> Result<()> {
        self.execute(op, opts).await.map(|_| ())
    }

    async fn execute(
        &self,
        op: Op,
        opts: &RequestOptions,
    ) -> Result<(HttpResponse, Option<String>)> {
        if let Some(e) = op.body_error {
            return Err(
                Error::validation(format!("could not encode the request: {e}")).with_source(e),
            );
        }
        let mut url = format!("{}{}", self.base_url, op.path);
        if !op.query.0.is_empty() {
            url = format!("{url}?{}", op.query.encode());
        }
        let key = match (&opts.idempotency_key, op.idempotent) {
            (Some(k), _) if !k.is_empty() => Some(k.clone()),
            (_, true) => Some(new_key()),
            _ => None,
        };
        let safe =
            op.method == Method::GET || op.method == Method::PUT || key.is_some() || op.retry_safe;
        let retries = opts.max_retries.unwrap_or(self.max_retries);

        let mut headers = vec![
            ("Accept".to_string(), "application/json".to_string()),
            (
                "Authorization".to_string(),
                format!("Bearer {}", self.api_key),
            ),
            (
                "User-Agent".to_string(),
                format!("gamebanking-sdk-rust/{VERSION}"),
            ),
        ];
        headers.extend(self.headers.iter().cloned());
        if op.body.is_some() {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        if let Some(k) = &key {
            headers.push(("Idempotency-Key".to_string(), k.clone()));
        }

        let mut attempt = 0u32;
        loop {
            let req = HttpRequest {
                method: op.method.clone(),
                url: url.clone(),
                headers: headers.clone(),
                body: op.body.clone(),
                timeout: self.timeout,
            };
            let err = match self.transport.send(req).await {
                Ok(res) => {
                    let rid = res
                        .header("x-request-id")
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    if let Some(hook) = &self.on_response {
                        hook(&ResponseInfo {
                            method: op.method.to_string(),
                            path: op.path.clone(),
                            status: res.status,
                            request_id: rid.clone(),
                            replayed: res.header("idempotent-replayed") == Some("true"),
                            attempt,
                        });
                    }
                    if (200..300).contains(&res.status) {
                        return Ok((res, rid));
                    }
                    to_error(&res, rid)
                }
                Err(TransportError::Timeout) => Error::local(
                    "timeout",
                    format!(
                        "no response from {} within {:?}",
                        self.base_url, self.timeout
                    ),
                )
                .with_source(TransportError::Timeout),
                Err(e) => Error::local(
                    "connection_error",
                    format!("could not reach the engine at {}: {e}", self.base_url),
                )
                .with_source(e),
            };
            if attempt >= retries || !should_retry(&err, safe) {
                return Err(err);
            }
            (self.sleep)(retry_delay(attempt, err.retry_after)).await;
            attempt += 1;
        }
    }
}

fn status_text(status: u16) -> String {
    reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("Unexpected status")
        .to_string()
}

fn to_error(res: &HttpResponse, rid: Option<String>) -> Error {
    let env: serde_json::Value = serde_json::from_slice(&res.body).unwrap_or_default();
    let e = &env["error"];
    let text = |k: &str| e[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    Error {
        status: res.status,
        code: text("code").unwrap_or_else(|| format!("http_{}", res.status)),
        message: text("message").unwrap_or_else(|| status_text(res.status)),
        details: e["details"].as_object().cloned().unwrap_or_default(),
        request_id: text("correlation_id").or(rid),
        retry_after: res
            .header("retry-after")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| *v > 0.0),
        ..Error::local("", "")
    }
}

fn should_retry(e: &Error, safe: bool) -> bool {
    if e.status == 429 {
        return true;
    }
    if !safe {
        return false;
    }
    match e.status {
        0 => e.code == "connection_error" || e.code == "timeout",
        500 | 502 | 503 | 504 => true,
        409 => e.code == "request_in_progress",
        _ => false,
    }
}

/// The wait before the next attempt: the engine's `Retry-After` (capped at
/// 60 s) when it sent one, else exponential backoff (0.5 s, 1 s, 2 s … capped
/// at 8 s) with jitter.
pub fn retry_delay(attempt: u32, retry_after_seconds: Option<f64>) -> Duration {
    if let Some(ra) = retry_after_seconds.filter(|v| *v > 0.0) {
        return Duration::from_secs_f64(ra.min(60.0));
    }
    let base = (0.5 * 2f64.powi(attempt.min(16) as i32)).min(8.0);
    Duration::from_secs_f64(base * (0.75 + rand::random::<f64>() * 0.25))
}

fn new_key() -> String {
    format!("sdk_{}", hex::encode(rand::random::<[u8; 16]>()))
}
