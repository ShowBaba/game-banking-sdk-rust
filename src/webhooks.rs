//! Signed event notifications: debit and credit alerts, card, savings and
//! fixed-deposit events. Verify each delivery with [`verify_webhook`].

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;
use sha2::Sha256;

use crate::client::{resource, Pager};
use crate::error::Result;
use crate::transport::{api_path, Inner, Op, Query};
use crate::types::{nullable, CurrencyBrief, List, Page, Timestamp};

/// The header that carries a delivery's timestamp and signatures.
pub const SIGNATURE_HEADER: &str = "GBE-Signature";

/// Event types an endpoint can subscribe to (`"*"` means all).
pub mod event_type {
    pub const ALL: &str = "*";
    pub const ACCOUNT_DEBITED: &str = "account.debited";
    pub const ACCOUNT_CREDITED: &str = "account.credited";
    pub const WEBHOOK_TEST: &str = "webhook.test";

    pub const CARD_ISSUED: &str = "card.issued";
    pub const CARD_BLOCKED: &str = "card.blocked";
    pub const CARD_UNBLOCKED: &str = "card.unblocked";
    pub const CARD_REPORTED_LOST: &str = "card.reported_lost";
    pub const CARD_CLOSED: &str = "card.closed";
    pub const CARD_EXPIRED: &str = "card.expired";
    pub const CARD_AUTHORIZATION_APPROVED: &str = "card.authorization.approved";
    pub const CARD_AUTHORIZATION_DECLINED: &str = "card.authorization.declined";
    pub const CARD_AUTHORIZATION_CAPTURED: &str = "card.authorization.captured";
    pub const CARD_AUTHORIZATION_RELEASED: &str = "card.authorization.released";
    pub const CARD_AUTHORIZATION_EXPIRED: &str = "card.authorization.expired";
    pub const CARD_REFUNDED: &str = "card.refunded";

    pub const INTEREST_PAID: &str = "interest.paid";
    pub const INTEREST_FORFEITED: &str = "interest.forfeited";
    pub const FIXED_DEPOSIT_OPENED: &str = "fixed_deposit.opened";
    pub const FIXED_DEPOSIT_MATURED: &str = "fixed_deposit.matured";
    pub const FIXED_DEPOSIT_ROLLED_OVER: &str = "fixed_deposit.rolled_over";
    pub const FIXED_DEPOSIT_BROKEN: &str = "fixed_deposit.broken";
    pub const SAVINGS_WITHDRAWAL_LIMIT_EXCEEDED: &str = "savings.withdrawal_limit_exceeded";
}

/// A registered receiver of signed events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookEndpoint {
    pub id: String,
    pub url: String,
    pub description: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub event_types: Vec<String>,
    /// `active` or `disabled`; the engine disables an endpoint after three days of failed deliveries.
    pub status: String,
    pub disabled_reason: Option<String>,
    pub secret_prefix: String,
    /// Set only when the endpoint is created or its secret rotated.
    pub secret: Option<String>,
    /// Until then deliveries are also signed with the previous secret.
    pub previous_secret_expires_at: Option<Timestamp>,
    pub failing_since: Option<Timestamp>,
    pub last_success_at: Option<Timestamp>,
    pub last_failure_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateWebhookEndpointParams {
    /// Must be https (the sandbox also accepts http); private and loopback addresses are refused.
    pub url: String,
    pub description: Option<String>,
    /// Defaults to `["*"]`.
    pub event_types: Option<Vec<String>>,
}

/// Changes an endpoint; `None` fields are unchanged.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateWebhookEndpointParams {
    pub url: Option<String>,
    pub description: Option<String>,
    pub event_types: Option<Vec<String>>,
    /// `active` re-enables an endpoint; `disabled` pauses it.
    pub status: Option<String>,
}

/// The body of a delivery, and an item of the events feed. Deduplicate on
/// `id`: delivery is at least once.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookEvent {
    pub id: String,
    pub r#type: String,
    pub created_at: Timestamp,
    pub game_time: Option<Timestamp>,
    pub game_id: String,
    pub environment: String,
    /// An [`AccountAlert`] for `account.*` events (see [`WebhookEvent::alert`]);
    /// otherwise the object the event is about.
    pub data: Value,
}

impl WebhookEvent {
    /// Decodes the data of an `account.debited` or `account.credited` event;
    /// `None` for other events.
    pub fn alert(&self) -> Option<std::result::Result<AccountAlert, serde_json::Error>> {
        if self.r#type != event_type::ACCOUNT_DEBITED && self.r#type != event_type::ACCOUNT_CREDITED
        {
            return None;
        }
        Some(serde_json::from_value(self.data.clone()))
    }

    /// Decodes the event's data into your own type.
    pub fn data_as<T: serde::de::DeserializeOwned>(
        &self,
    ) -> std::result::Result<T, serde_json::Error> {
        serde_json::from_value(self.data.clone())
    }
}

/// A fee or levy line on an alert: charged on a debit, refunded on a credit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertCharge {
    pub kind: String,
    pub name: String,
    pub fee_rule_id: Option<String>,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertBalance {
    pub posted_minor: i64,
    pub available_minor: i64,
}

/// The other side of an alert. Customer account numbers are masked to the last four digits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertCounterparty {
    pub r#type: String,
    pub account_name: String,
    pub account_number: String,
    pub bank_code: String,
}

/// The data of an `account.debited` or `account.credited` event.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountAlert {
    pub account_id: String,
    pub account_number: String,
    pub account_name: String,
    pub player_id: Option<String>,
    pub bank_id: String,
    pub bank_code: String,
    pub transaction_id: String,
    pub session_id: String,
    pub transaction_type: String,
    pub related_transaction_id: Option<String>,
    /// The principal (0 for a fee-only transaction).
    pub amount_minor: i64,
    #[serde(deserialize_with = "nullable")]
    pub charges: Vec<AlertCharge>,
    /// Principal plus charges: what left (or reached) the account.
    pub total_minor: i64,
    pub total_display: String,
    pub currency: CurrencyBrief,
    pub balance_after: AlertBalance,
    pub narration: Option<String>,
    pub reference: Option<String>,
    pub counterparty: Option<AlertCounterparty>,
    pub game_time: Timestamp,
    /// Increases with every ledger entry: keep the alert with the highest for the latest balance.
    pub sequence: i64,
}

/// One event's delivery to one endpoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookDelivery {
    pub id: String,
    pub event_id: String,
    pub event_type: String,
    pub endpoint_id: String,
    pub endpoint_url: String,
    /// `pending`, `delivering`, `delivered` or `failed`.
    pub state: String,
    pub attempts: i32,
    pub next_attempt_at: Option<Timestamp>,
    pub last_attempt_at: Option<Timestamp>,
    /// The endpoint's HTTP status on the last attempt.
    pub last_status: Option<i32>,
    pub last_error: Option<String>,
    pub last_response_ms: Option<i64>,
    pub delivered_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// One page of the event log.
pub type WebhookEventPage = Page<WebhookEvent>;

#[derive(Debug, Clone, Default)]
pub struct ListWebhookEventsParams {
    pub r#type: Option<String>,
    pub account_id: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// Signed event notifications. Verify deliveries with [`verify_webhook`].
#[derive(Clone)]
pub struct Webhooks {
    t: Arc<Inner>,
}

impl fmt::Debug for Webhooks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Webhooks")
    }
}

impl Webhooks {
    pub(crate) fn new(t: &Arc<Inner>) -> Self {
        Webhooks { t: t.clone() }
    }

    pub fn endpoints(&self) -> WebhookEndpoints {
        WebhookEndpoints::new(&self.t)
    }

    pub fn events(&self) -> WebhookEvents {
        WebhookEvents::new(&self.t)
    }
}

resource!(WebhookEndpoints);

impl WebhookEndpoints {
    /// Registers an endpoint. The result's `secret` is shown only now: store
    /// it and pass it to [`verify_webhook`]. Needs `webhooks:write`.
    pub async fn create(&self, p: &CreateWebhookEndpointParams) -> Result<WebhookEndpoint> {
        self.t
            .call(
                Op::post("/v1/webhooks/endpoints".into()).json(p),
                &self.opts,
            )
            .await
    }

    pub async fn list(&self) -> Result<Vec<WebhookEndpoint>> {
        let out: List<WebhookEndpoint> = self
            .t
            .call(Op::get("/v1/webhooks/endpoints".into()), &self.opts)
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, endpoint_id: &str) -> Result<WebhookEndpoint> {
        self.t
            .call(
                Op::get(api_path!("/v1/webhooks/endpoints/{}", endpoint_id)),
                &self.opts,
            )
            .await
    }

    /// Changes the URL, description or event types, or pauses (`disabled`) and re-enables it.
    pub async fn update(
        &self,
        endpoint_id: &str,
        p: &UpdateWebhookEndpointParams,
    ) -> Result<WebhookEndpoint> {
        self.t
            .call(
                Op::patch(api_path!("/v1/webhooks/endpoints/{}", endpoint_id)).json(p),
                &self.opts,
            )
            .await
    }

    pub async fn delete(&self, endpoint_id: &str) -> Result<()> {
        self.t
            .call_empty(
                Op::delete(api_path!("/v1/webhooks/endpoints/{}", endpoint_id)),
                &self.opts,
            )
            .await
    }

    /// Issues a new secret (in the result). The old one keeps signing for 24 hours.
    pub async fn rotate_secret(&self, endpoint_id: &str) -> Result<WebhookEndpoint> {
        self.t
            .call(
                Op::post(api_path!(
                    "/v1/webhooks/endpoints/{}/rotate-secret",
                    endpoint_id
                )),
                &self.opts,
            )
            .await
    }

    /// Sends a `webhook.test` event now and returns how that attempt went.
    pub async fn test(&self, endpoint_id: &str) -> Result<WebhookDelivery> {
        self.t
            .call(
                Op::post(api_path!("/v1/webhooks/endpoints/{}/test", endpoint_id)),
                &self.opts,
            )
            .await
    }
}

resource!(WebhookEvents);

impl WebhookEvents {
    /// One page of the event log, newest first (kept 30 days). It is also a polling feed.
    pub async fn list(&self, p: &ListWebhookEventsParams) -> Result<WebhookEventPage> {
        let q = Query::new()
            .opt("type", p.r#type.as_ref())
            .opt("account_id", p.account_id.as_ref())
            .opt("limit", p.limit)
            .opt("cursor", p.cursor.as_ref());
        self.t
            .call(Op::get("/v1/webhooks/events".into()).query(q), &self.opts)
            .await
    }

    /// Every event matching the filter, newest first, fetching pages as needed.
    pub fn iterate(&self, p: &ListWebhookEventsParams) -> Pager<WebhookEvent> {
        let this = self.clone();
        let p = p.clone();
        Pager::new(move |cursor| {
            let (this, mut p) = (this.clone(), p.clone());
            p.cursor = cursor;
            Box::pin(async move { this.list(&p).await })
        })
    }

    pub async fn get(&self, event_id: &str) -> Result<WebhookEvent> {
        self.t
            .call(
                Op::get(api_path!("/v1/webhooks/events/{}", event_id)),
                &self.opts,
            )
            .await
    }

    /// The event's deliveries, one per endpoint, with attempts and the last error.
    pub async fn deliveries(&self, event_id: &str) -> Result<Vec<WebhookDelivery>> {
        let out: List<WebhookDelivery> = self
            .t
            .call(
                Op::get(api_path!("/v1/webhooks/events/{}/deliveries", event_id)),
                &self.opts,
            )
            .await?;
        Ok(out.data)
    }

    /// Sends the event again, to one endpoint or (`None`) every subscribed one. Retries start over.
    pub async fn redeliver(
        &self,
        event_id: &str,
        endpoint_id: Option<&str>,
    ) -> Result<Vec<WebhookDelivery>> {
        let body: std::collections::BTreeMap<_, _> = endpoint_id
            .map(|e| ("endpoint_id", e))
            .into_iter()
            .collect();
        let out: List<WebhookDelivery> = self
            .t
            .call(
                Op::post(api_path!("/v1/webhooks/events/{}/redeliver", event_id)).json(&body),
                &self.opts,
            )
            .await?;
        Ok(out.data)
    }
}

// ---- verification ----

/// Why a delivery failed verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerificationFailure {
    /// The signature header is missing or malformed.
    MalformedHeader,
    /// The signature's timestamp is outside the tolerance (a replay, or a clock problem).
    TimestampOutOfTolerance,
    /// No signature matches the secret.
    NoMatchingSignature,
    /// The body is not a JSON event.
    InvalidPayload,
}

impl VerificationFailure {
    /// `malformed_header`, `timestamp_out_of_tolerance`, `no_matching_signature` or `invalid_payload`.
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationFailure::MalformedHeader => "malformed_header",
            VerificationFailure::TimestampOutOfTolerance => "timestamp_out_of_tolerance",
            VerificationFailure::NoMatchingSignature => "no_matching_signature",
            VerificationFailure::InvalidPayload => "invalid_payload",
        }
    }
}

/// The request did not come from the engine, or was replayed or tampered
/// with. Answer 400.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookVerificationError {
    pub reason: VerificationFailure,
    pub message: String,
}

impl WebhookVerificationError {
    fn new(reason: VerificationFailure, message: &str) -> Self {
        WebhookVerificationError {
            reason,
            message: message.to_string(),
        }
    }
}

impl fmt::Display for WebhookVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gamebank: {}", self.message)
    }
}

impl std::error::Error for WebhookVerificationError {}

/// Checks deliveries against one or more secrets (several while you roll a
/// new one out).
#[derive(Debug, Clone)]
pub struct WebhookVerifier {
    secrets: Vec<String>,
    tolerance: Duration,
    now: Option<SystemTime>,
}

impl WebhookVerifier {
    /// A verifier with the default 5-minute tolerance.
    pub fn new<S: Into<String>>(secrets: impl IntoIterator<Item = S>) -> Self {
        WebhookVerifier {
            secrets: secrets.into_iter().map(Into::into).collect(),
            tolerance: Duration::from_secs(300),
            now: None,
        }
    }

    /// How far the signature's timestamp may be from now.
    pub fn tolerance(mut self, tolerance: Duration) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Fixes the current time, for tests.
    pub fn now(mut self, now: SystemTime) -> Self {
        self.now = Some(now);
        self
    }

    /// Checks that a signature matches one of the secrets and that its
    /// timestamp is within the tolerance, then decodes the event. Pass the
    /// raw body, before any JSON decoding.
    pub fn verify(
        &self,
        payload: &[u8],
        signature_header: &str,
    ) -> std::result::Result<WebhookEvent, WebhookVerificationError> {
        let mut ts: i64 = 0;
        let mut sigs: Vec<Vec<u8>> = Vec::new();
        for part in signature_header.split(',') {
            let Some((k, v)) = part.trim().split_once('=') else {
                continue;
            };
            match k {
                "t" => {
                    if let Ok(n) = v.parse::<i64>() {
                        ts = n;
                    }
                }
                "v1" => {
                    if let Ok(b) = hex::decode(v) {
                        sigs.push(b);
                    }
                }
                _ => {}
            }
        }
        if ts <= 0 || sigs.is_empty() {
            return Err(WebhookVerificationError::new(
                VerificationFailure::MalformedHeader,
                "missing or malformed GBE-Signature header",
            ));
        }
        let now = self
            .now
            .unwrap_or_else(SystemTime::now)
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        if (now - ts as f64).abs() > self.tolerance.as_secs_f64() {
            return Err(WebhookVerificationError::new(
                VerificationFailure::TimestampOutOfTolerance,
                "the signature's timestamp is outside the tolerance",
            ));
        }
        let prefix = format!("{ts}.");
        let matched = self.secrets.iter().any(|secret| {
            sigs.iter().any(|sig| {
                let mut m = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                    .expect("HMAC takes any key length");
                m.update(prefix.as_bytes());
                m.update(payload);
                m.verify_slice(sig).is_ok() // constant time
            })
        });
        if !matched {
            return Err(WebhookVerificationError::new(
                VerificationFailure::NoMatchingSignature,
                "no signature matches the secret",
            ));
        }
        serde_json::from_slice(payload).map_err(|_| {
            WebhookVerificationError::new(
                VerificationFailure::InvalidPayload,
                "the body is not a JSON event",
            )
        })
    }
}

/// Checks a delivery with one secret and the default 5-minute tolerance, and
/// returns the event. Pass the raw body, before any JSON decoding, and the
/// `GBE-Signature` header:
///
/// ```no_run
/// # fn handle(body: &[u8], header: &str, secret: &str) {
/// match gamebanking::verify_webhook(body, header, secret) {
///     Ok(event) => println!("{} {}", event.r#type, event.id),
///     Err(e) => eprintln!("rejected: {}", e.reason.as_str()), // answer 400
/// }
/// # }
/// ```
pub fn verify_webhook(
    payload: &[u8],
    signature_header: &str,
    secret: &str,
) -> std::result::Result<WebhookEvent, WebhookVerificationError> {
    WebhookVerifier::new([secret]).verify(payload, signature_header)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"hello":"world"}"#;
    const SIG: &str = "f592bbf3951cfc94e560eecfb5d9dd4da6b0fff2e626235f8ab4b54860925d0b";

    fn at(unix: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(unix)
    }

    fn sign(secret: &str, ts: i64, body: &str) -> String {
        let mut m = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        m.update(format!("{ts}.{body}").as_bytes());
        hex::encode(m.finalize().into_bytes())
    }

    fn reason(r: std::result::Result<WebhookEvent, WebhookVerificationError>) -> &'static str {
        r.expect_err("verification should fail").reason.as_str()
    }

    #[test]
    fn engine_test_vector() {
        assert_eq!(sign("whsec_test", 1_700_000_000, BODY), SIG);
        let v = WebhookVerifier::new(["whsec_test"]).now(at(1_700_000_010));
        v.verify(BODY.as_bytes(), &format!("t=1700000000,v1={SIG}"))
            .unwrap();
        // Any matching v1, and any of several secrets.
        let header = format!(
            "t=1700000000,v1={},v1={SIG}",
            sign("whsec_new", 1_700_000_000, BODY)
        );
        for s in ["whsec_new", "whsec_test"] {
            WebhookVerifier::new([s])
                .now(at(1_700_000_010))
                .verify(BODY.as_bytes(), &header)
                .unwrap();
        }
        WebhookVerifier::new(["whsec_other", "whsec_test"])
            .now(at(1_700_000_010))
            .verify(BODY.as_bytes(), &format!("t=1700000000,v1={SIG}"))
            .unwrap();
    }

    #[test]
    fn rejections() {
        let h = format!("t=1700000000,v1={SIG}");
        let v = WebhookVerifier::new(["whsec_test"]).now(at(1_700_000_010));
        assert_eq!(reason(v.verify(BODY.as_bytes(), "")), "malformed_header");
        assert_eq!(
            reason(v.verify(BODY.as_bytes(), "t=1700000000")),
            "malformed_header"
        );
        assert_eq!(
            reason(v.verify(BODY.as_bytes(), &format!("v1={SIG}"))),
            "malformed_header"
        );
        assert_eq!(
            reason(v.verify(BODY.as_bytes(), "t=1700000000,v1=zz")),
            "malformed_header"
        );
        assert_eq!(
            reason(v.verify(br#"{"hello":"World"}"#, &h)),
            "no_matching_signature"
        );
        assert_eq!(
            reason(
                WebhookVerifier::new(["whsec_wrong"])
                    .now(at(1_700_000_010))
                    .verify(BODY.as_bytes(), &h)
            ),
            "no_matching_signature"
        );
        assert_eq!(
            reason(
                WebhookVerifier::new(["whsec_test"])
                    .now(at(1_700_000_301))
                    .verify(BODY.as_bytes(), &h)
            ),
            "timestamp_out_of_tolerance"
        );
        assert_eq!(
            reason(
                WebhookVerifier::new(["whsec_test"])
                    .now(at(1_699_999_699))
                    .verify(BODY.as_bytes(), &h)
            ),
            "timestamp_out_of_tolerance"
        );
        WebhookVerifier::new(["whsec_test"])
            .now(at(1_700_000_600))
            .tolerance(Duration::from_secs(600))
            .verify(BODY.as_bytes(), &h)
            .unwrap();
        let bad = "not json";
        let hb = format!("t=1700000000,v1={}", sign("whsec_test", 1_700_000_000, bad));
        assert_eq!(reason(v.verify(bad.as_bytes(), &hb)), "invalid_payload");
    }

    #[test]
    fn decodes_alerts() {
        let body = r#"{"id":"evt_1","type":"account.debited","created_at":"2026-01-01T00:00:00Z","game_time":null,
            "game_id":"g","environment":"sandbox","data":{"account_id":"a1","amount_minor":500,"total_minor":550,
            "charges":[{"kind":"fee","name":"Transfer fee","fee_rule_id":null,"amount_minor":50}],
            "balance_after":{"posted_minor":1000,"available_minor":900},"counterparty":null,"sequence":7,
            "currency":{"code":"NGN","exponent":2,"symbol":null},"game_time":"2026-01-01T00:00:00Z"}}"#;
        let ts = 1_700_000_000;
        let v = WebhookVerifier::new(["s"]).now(at(ts as u64));
        let ev = v
            .verify(
                body.as_bytes(),
                &format!("t={ts},v1={}", sign("s", ts, body)),
            )
            .unwrap();
        let alert = ev.alert().unwrap().unwrap();
        assert_eq!(
            (
                alert.account_id.as_str(),
                alert.total_minor,
                alert.charges.len()
            ),
            ("a1", 550, 1)
        );
        assert_eq!(alert.balance_after.available_minor, 900);
        let other = WebhookEvent {
            r#type: event_type::CARD_ISSUED.into(),
            ..Default::default()
        };
        assert!(other.alert().is_none());
    }
}
