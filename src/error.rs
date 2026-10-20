use std::fmt;

use serde_json::{Map, Value};

/// The result of every call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Error categories. Match on [`Error::kind`] for these, and on
/// [`Error::code`] (the engine's stable error code) for anything finer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// 401.
    Authentication,
    /// 403: a missing scope, a suspended game, …
    PermissionDenied,
    /// 404.
    NotFound,
    /// 409: `idempotency_key_reused`, `email_taken`, `already_reversed`, …
    Conflict,
    /// 409 `fee_changed`: the fees now total more than the request's `max_fee_minor`;
    /// `details["fees"]` lists the current charges. Also a conflict.
    FeeChanged,
    /// 409 `penalty_changed`: a fixed deposit's early-break penalty now exceeds
    /// `max_penalty_minor`. Also a conflict.
    PenaltyChanged,
    /// 400, 413, 415, or a request the SDK refused to send.
    InvalidRequest,
    /// 422 `insufficient_funds`. Also unprocessable.
    InsufficientFunds,
    /// 422 `limit_exceeded`. Also unprocessable.
    LimitExceeded,
    /// Any other 422 business rule (`account_frozen`, `hold_not_active`, …).
    Unprocessable,
    /// 429.
    RateLimited,
    /// 5xx.
    Server,
    /// No response: a network failure or timeout (`status` 0).
    Connection,
    /// The engine answered with something that is not the expected JSON, e.g.
    /// because the base URL points at a website.
    InvalidResponse,
    /// The client is missing its API key or base URL.
    Configuration,
}

/// Any failure from the Game Banking Engine or from reaching it.
///
/// ```no_run
/// # async fn run(gb: gamebanking::GameBank, p: gamebanking::TransferParams) {
/// use gamebanking::ErrorKind;
/// match gb.transfers().create(&p).await {
///     Ok(r) => println!("sent {}", r.transaction.id),
///     Err(e) if e.kind() == ErrorKind::InsufficientFunds => {
///         println!("available {:?} (request {:?})", e.details.get("available_balance"), e.request_id)
///     }
///     Err(e) => eprintln!("{e}"),
/// }
/// # }
/// ```
#[derive(Debug)]
pub struct Error {
    /// The HTTP status, or 0 when no response arrived.
    pub status: u16,
    /// The engine's stable error code, e.g. `insufficient_funds`.
    pub code: String,
    pub message: String,
    /// Extra context, e.g. `available_balance`.
    pub details: Map<String, Value>,
    /// The engine's correlation ID (`X-Request-Id`).
    pub request_id: Option<String>,
    /// The number of seconds the engine asked the caller to wait.
    pub retry_after: Option<f64>,
    pub(crate) source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    pub(crate) fn local(code: &str, message: impl Into<String>) -> Self {
        Error {
            status: 0,
            code: code.to_string(),
            message: message.into(),
            details: Map::new(),
            request_id: None,
            retry_after: None,
            source: None,
        }
    }

    pub(crate) fn with_source(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub(crate) fn validation(message: impl Into<String>) -> Self {
        Error::local("validation_error", message)
    }

    /// The error's category.
    pub fn kind(&self) -> ErrorKind {
        match (self.status, self.code.as_str()) {
            (0, "connection_error" | "timeout") => ErrorKind::Connection,
            (0, "missing_api_key") => ErrorKind::Configuration,
            (0, _) => ErrorKind::InvalidRequest,
            (_, "invalid_response") => ErrorKind::InvalidResponse,
            (401, _) => ErrorKind::Authentication,
            (403, _) => ErrorKind::PermissionDenied,
            (404, _) => ErrorKind::NotFound,
            (409, "fee_changed") => ErrorKind::FeeChanged,
            (409, "penalty_changed") => ErrorKind::PenaltyChanged,
            (409, _) => ErrorKind::Conflict,
            (422, "insufficient_funds") => ErrorKind::InsufficientFunds,
            (422, "limit_exceeded") => ErrorKind::LimitExceeded,
            (422, _) => ErrorKind::Unprocessable,
            (429, _) => ErrorKind::RateLimited,
            (s, _) if s >= 500 => ErrorKind::Server,
            _ => ErrorKind::InvalidRequest,
        }
    }

    /// True for every 409, including [`ErrorKind::FeeChanged`] and [`ErrorKind::PenaltyChanged`].
    pub fn is_conflict(&self) -> bool {
        self.status == 409
    }

    /// True for every 422, including [`ErrorKind::InsufficientFunds`] and [`ErrorKind::LimitExceeded`].
    pub fn is_unprocessable(&self) -> bool {
        self.status == 422
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gamebank: {}: {}", self.code, self.message)?;
        if let Some(rid) = &self.request_id {
            write!(f, " (request {rid})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}
