//! Unit tests against a scripted transport.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::cards::{AuthorizeCardParams, CreateMerchantParams, RefundCardParams};
use crate::savings::OpenFixedDepositParams;
use crate::*;

/// A scripted response; `Err` simulates a network failure.
type Reply = std::result::Result<HttpResponse, TransportError>;

#[derive(Default)]
struct Script {
    replies: VecDeque<Reply>,
    calls: Vec<HttpRequest>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Script>>);

impl HttpTransport for Fake {
    fn send(
        &self,
        req: HttpRequest,
    ) -> BoxFuture<'_, std::result::Result<HttpResponse, TransportError>> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(req);
        let reply = s.replies.pop_front().unwrap_or_else(|| {
            Err(TransportError::Connection(
                "no more scripted replies".into(),
            ))
        });
        Box::pin(async move { reply })
    }
}

impl Fake {
    fn calls(&self) -> Vec<HttpRequest> {
        self.0.lock().unwrap().calls.clone()
    }
}

fn reply(status: u16, body: Value) -> Reply {
    Ok(HttpResponse {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_vec(&body).unwrap(),
    })
}

fn api_err(status: u16, code: &str, details: Value) -> Reply {
    reply(
        status,
        json!({"error": {"code": code, "message": code, "details": details, "correlation_id": "req_1"}}),
    )
}

fn reset() -> Reply {
    Err(TransportError::Connection("connection reset".into()))
}

fn transfer_ok() -> Reply {
    reply(
        201,
        json!({"transaction": {"id": "t1"}, "source_balance": {}}),
    )
}

/// A client on a scripted transport that records retry waits instead of sleeping.
fn fake(replies: Vec<Reply>, max_retries: u32) -> (GameBank, Fake, Arc<Mutex<Vec<Duration>>>) {
    let f = Fake::default();
    f.0.lock().unwrap().replies = replies.into();
    let mut gb = GameBank::builder()
        .api_key("gbe_test_sk_abc")
        .base_url("http://engine.test/")
        .max_retries(max_retries)
        .transport(f.clone())
        .build()
        .unwrap();
    let waits = Arc::new(Mutex::new(Vec::new()));
    let w = waits.clone();
    Arc::get_mut(&mut gb.t).unwrap().sleep = Arc::new(move |d| {
        w.lock().unwrap().push(d);
        Box::pin(async {})
    });
    (gb, f, waits)
}

fn header<'a>(r: &'a HttpRequest, name: &str) -> Option<&'a str> {
    r.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn route(r: &HttpRequest) -> String {
    let path = r.url.trim_start_matches("http://engine.test");
    format!("{} {}", r.method, path.split('?').next().unwrap())
}

fn transfer(amount: i64) -> TransferParams {
    TransferParams {
        source_account_id: "a".into(),
        destination_account_id: Some("b".into()),
        amount_minor: amount,
        ..Default::default()
    }
}

#[tokio::test]
async fn configuration() {
    let (gb, f, _) = fake(vec![reply(200, json!({"mode": "manual"}))], 2);
    assert_eq!(gb.environment(), Some(Environment::Sandbox));
    assert_eq!(gb.clock().get().await.unwrap().mode, "manual");
    let call = &f.calls()[0];
    assert_eq!(call.url, "http://engine.test/v1/clock");
    assert_eq!(
        header(call, "authorization"),
        Some("Bearer gbe_test_sk_abc")
    );
    assert_eq!(
        header(call, "user-agent"),
        Some(&*format!("gamebanking-sdk-rust/{VERSION}"))
    );
    assert_eq!(call.timeout, Duration::from_secs(30));
}

#[test]
fn configuration_from_env() {
    // One test owns the environment variables, so runs cannot interfere.
    std::env::remove_var("GBE_API_KEY");
    std::env::remove_var("GBE_BASE_URL");
    let e = GameBank::builder()
        .base_url("http://e")
        .build()
        .unwrap_err();
    assert_eq!(
        (e.kind(), e.code.as_str()),
        (ErrorKind::Configuration, "missing_api_key")
    );
    let e = GameBank::builder()
        .api_key("gbe_test_sk_x")
        .build()
        .unwrap_err();
    assert_eq!(e.code, "missing_base_url");
    std::env::set_var("GBE_API_KEY", "gbe_live_sk_env");
    std::env::set_var("GBE_BASE_URL", "http://env");
    assert_eq!(
        GameBank::from_env().unwrap().environment(),
        Some(Environment::Production)
    );
    assert!(GameBankManagement::from_env().is_ok());
    std::env::remove_var("GBE_API_KEY");
    std::env::remove_var("GBE_BASE_URL");
    assert_eq!(Environment::of("gbe_dev_x"), None);
}

#[tokio::test]
async fn idempotency_key_reused_across_retries() {
    let (gb, f, _) = fake(vec![reset(), reply(503, json!({})), transfer_ok()], 2);
    let r = gb.transfers().create(&transfer(100)).await.unwrap();
    assert_eq!(r.transaction.id, "t1");
    let calls = f.calls();
    assert_eq!(calls.len(), 3);
    let key = header(&calls[0], "idempotency-key").unwrap();
    assert!(key.starts_with("sdk_") && key.len() == 36, "{key}");
    assert!(calls
        .iter()
        .all(|c| header(c, "idempotency-key") == Some(key)));
}

#[tokio::test]
async fn caller_key_used() {
    let (gb, f, _) = fake(vec![transfer_ok()], 2);
    gb.transfers()
        .idempotency_key("order-42")
        .create(&transfer(1))
        .await
        .unwrap();
    assert_eq!(header(&f.calls()[0], "idempotency-key"), Some("order-42"));
}

#[tokio::test]
async fn plain_post_not_retried() {
    for first in [reset(), reply(502, json!({}))] {
        let (gb, f, _) = fake(vec![first, reply(201, json!({}))], 2);
        let p = OpenAccountParams {
            bank_id: "b".into(),
            player_id: Some("p".into()),
            ..Default::default()
        };
        assert!(gb.accounts().open(&p).await.is_err());
        let calls = f.calls();
        assert_eq!(calls.len(), 1, "accounts.open must not be retried");
        assert_eq!(header(&calls[0], "idempotency-key"), None);
    }
}

#[tokio::test]
async fn rate_limit_honours_retry_after() {
    let limited = Ok(HttpResponse {
        status: 429,
        headers: vec![("Retry-After".into(), "3".into())],
        body: br#"{"error":{"code":"rate_limited"}}"#.to_vec(),
    });
    let (gb, f, waits) = fake(vec![limited, reply(201, json!({"id": "acc"}))], 2);
    let p = OpenAccountParams {
        bank_id: "b".into(),
        player_id: Some("p".into()),
        ..Default::default()
    };
    assert_eq!(gb.accounts().open(&p).await.unwrap().id, "acc");
    assert_eq!(f.calls().len(), 2);
    assert_eq!(*waits.lock().unwrap(), vec![Duration::from_secs(3)]);
}

#[tokio::test]
async fn gets_and_upserts_retried() {
    let (gb, f, waits) = fake(
        vec![
            reset(),
            reply(200, json!({"id": "p1"})),
            reset(),
            reply(200, json!({"id": "p2"})),
        ],
        2,
    );
    assert_eq!(gb.players().get("p1").await.unwrap().id, "p1");
    let p = UpsertPlayerParams {
        external_player_id: "x".into(),
        ..Default::default()
    };
    assert_eq!(gb.players().upsert(&p).await.unwrap().id, "p2");
    assert_eq!(f.calls().len(), 4);
    for w in waits.lock().unwrap().iter() {
        assert!(
            *w >= Duration::from_millis(375) && *w <= Duration::from_millis(500),
            "{w:?}"
        );
    }
}

#[tokio::test]
async fn stops_after_max_retries() {
    let (gb, f, _) = fake(
        vec![
            reply(500, json!({})),
            reply(500, json!({})),
            reply(500, json!({})),
        ],
        1,
    );
    let e = gb.accounts().balance("a").await.unwrap_err();
    assert_eq!((e.kind(), e.code.as_str()), (ErrorKind::Server, "http_500"));
    assert_eq!(f.calls().len(), 2);
    // A per-call override.
    let (gb, f, _) = fake(vec![reply(500, json!({})), reply(200, json!({}))], 2);
    assert!(gb.accounts().max_retries(0).balance("a").await.is_err());
    assert_eq!(f.calls().len(), 1);
}

#[tokio::test]
async fn request_in_progress_retried_for_keyed_requests() {
    let (gb, f, _) = fake(
        vec![
            api_err(409, "request_in_progress", Value::Null),
            transfer_ok(),
        ],
        2,
    );
    gb.transfers().create(&transfer(1)).await.unwrap();
    assert_eq!(f.calls().len(), 2);
}

#[tokio::test]
async fn typed_errors() {
    let (gb, _, _) = fake(
        vec![api_err(
            422,
            "insufficient_funds",
            json!({"available_balance": 5}),
        )],
        2,
    );
    let e = gb.transfers().create(&transfer(9)).await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InsufficientFunds);
    assert!(e.is_unprocessable() && !e.is_conflict());
    assert_eq!(e.details["available_balance"], json!(5));
    assert_eq!((e.request_id.as_deref(), e.status), (Some("req_1"), 422));
    assert_eq!(
        e.to_string(),
        "gamebank: insufficient_funds: insufficient_funds (request req_1)"
    );

    let (gb, _, _) = fake(vec![api_err(409, "idempotency_key_reused", Value::Null)], 2);
    let e = gb.transfers().create(&transfer(9)).await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Conflict);

    let (gb, f, _) = fake(
        vec![api_err(409, "fee_changed", json!({"fees_total_minor": 70}))],
        2,
    );
    let e = gb
        .transfers()
        .create(&TransferParams {
            max_fee_minor: Some(60),
            ..transfer(9)
        })
        .await
        .unwrap_err();
    assert!(e.kind() == ErrorKind::FeeChanged && e.is_conflict());
    let body = String::from_utf8(f.calls()[0].body.clone().unwrap()).unwrap();
    assert!(body.contains(r#""max_fee_minor":60"#), "{body}");
    assert!(
        !body.contains("null"),
        "unset fields must be left out: {body}"
    );

    for (status, kind) in [
        (401, ErrorKind::Authentication),
        (403, ErrorKind::PermissionDenied),
        (404, ErrorKind::NotFound),
        (400, ErrorKind::InvalidRequest),
        (413, ErrorKind::InvalidRequest),
        (422, ErrorKind::Unprocessable),
    ] {
        let (gb, _, _) = fake(vec![api_err(status, "x", Value::Null)], 0);
        assert_eq!(gb.clock().get().await.unwrap_err().kind(), kind, "{status}");
    }
}

#[tokio::test]
async fn connection_error_after_retries() {
    let (gb, _, _) = fake(vec![reset(), Err(TransportError::Timeout)], 1);
    let e = gb.clock().get().await.unwrap_err();
    assert_eq!(
        (e.kind(), e.code.as_str(), e.status),
        (ErrorKind::Connection, "timeout", 0)
    );
    assert!(std::error::Error::source(&e).is_some());
}

#[tokio::test]
async fn html_instead_of_engine() {
    let html = Ok(HttpResponse {
        status: 200,
        headers: vec![("Content-Type".into(), "text/html".into())],
        body: b"<!doctype html>".to_vec(),
    });
    let (gb, _, _) = fake(vec![html], 2);
    let e = gb.clock().get().await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidResponse);
    assert!(
        e.message.contains("base URL") && e.message.contains("text/html"),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn bad_amount_rejected_before_sending() {
    let (gb, f, _) = fake(vec![], 2);
    let p = FundingParams {
        account_id: "a".into(),
        amount_minor: -1,
        reason: "r".into(),
        reference: "x".into(),
        ..Default::default()
    };
    let e = gb.funding().create(&p).await.unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidRequest);
    let e = gb
        .card_authorizations()
        .authorize(&AuthorizeCardParams {
            card_id: Some("c1".into()),
            ..Default::default()
        })
        .await;
    assert_eq!(e.unwrap_err().code, "validation_error");
    assert!(f.calls().is_empty());
}

#[tokio::test]
async fn response_hook_and_escaped_paths() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let f = Fake::default();
    f.0.lock().unwrap().replies = vec![Ok(HttpResponse {
        status: 200,
        headers: vec![
            ("X-Request-Id".into(), "req_9".into()),
            ("Idempotent-Replayed".into(), "true".into()),
        ],
        body: br#"{"id":"a/b"}"#.to_vec(),
    })]
    .into();
    let gb = GameBank::builder()
        .api_key("k")
        .base_url("http://engine.test")
        .transport(f.clone())
        .on_response(move |r| s.lock().unwrap().push(r.clone()))
        .build()
        .unwrap();
    assert_eq!(gb.environment(), None);
    gb.accounts().get("a/b c").await.unwrap();
    assert_eq!(f.calls()[0].url, "http://engine.test/v1/accounts/a%2Fb%20c");
    let seen = seen.lock().unwrap();
    assert_eq!(
        (
            seen[0].status,
            seen[0].request_id.as_deref(),
            seen[0].replayed
        ),
        (200, Some("req_9"), true)
    );
}

#[tokio::test]
async fn pagination_follows_cursors() {
    let (gb, f, _) = fake(
        vec![
            reply(
                200,
                json!({"data": [{"amount_minor": 1}, {"amount_minor": 2}], "next_cursor": "c2", "has_more": true}),
            ),
            reply(
                200,
                json!({"data": [{"amount_minor": 3}], "next_cursor": null, "has_more": false}),
            ),
        ],
        2,
    );
    let items = gb
        .accounts()
        .iterate_transactions("acc", 2)
        .collect()
        .await
        .unwrap();
    assert_eq!(
        items.iter().map(|i| i.amount_minor).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let calls = f.calls();
    assert_eq!(
        calls[0].url,
        "http://engine.test/v1/accounts/acc/transactions?limit=2"
    );
    assert_eq!(
        calls[1].url,
        "http://engine.test/v1/accounts/acc/transactions?limit=2&cursor=c2"
    );
}

#[tokio::test]
async fn null_lists_decode_as_empty() {
    let (gb, _, _) = fake(
        vec![reply(
            201,
            json!({"transaction": {"id": "t", "entries": null, "metadata": null}, "fees": null}),
        )],
        2,
    );
    let r = gb.transfers().create(&transfer(1)).await.unwrap();
    assert!(
        r.fees.is_empty() && r.transaction.entries.is_empty() && r.destination_balance.is_none()
    );
}

#[tokio::test]
async fn card_decline_is_a_result_and_routes() {
    let (gb, f, _) = fake(
        vec![
            reply(
                201,
                json!({"authorization": {"id": "a1", "status": "declined", "decline_code": "card_blocked"}}),
            ),
            reply(200, json!({"id": "c1", "status": "stolen"})),
            reply(201, json!({"refund": {"id": "r1"}})),
            reply(201, json!({"id": "m1"})),
        ],
        2,
    );
    let r = gb
        .card_authorizations()
        .authorize(&AuthorizeCardParams {
            card_id: Some("c1".into()),
            merchant_account_id: Some("ma1".into()),
            amount_minor: 500,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        r.authorization.declined(),
        Some(cards::decline_code::CARD_BLOCKED)
    );
    gb.cards().report_lost("c1", "stolen").await.unwrap();
    gb.card_authorizations()
        .refund(
            "a1",
            &RefundCardParams {
                amount_minor: Some(100),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    gb.merchants()
        .create(&CreateMerchantParams {
            external_merchant_id: "shop-1".into(),
            name: "Shop".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let calls = f.calls();
    assert_eq!(
        calls.iter().map(route).collect::<Vec<_>>(),
        [
            "POST /v1/card-authorizations",
            "POST /v1/cards/c1/report-lost",
            "POST /v1/card-authorizations/a1/refunds",
            "POST /v1/merchants"
        ]
    );
    assert!(
        header(&calls[0], "idempotency-key").is_some()
            && header(&calls[2], "idempotency-key").is_some()
    );
    assert!(header(&calls[3], "idempotency-key").is_none());
    let body: Value = serde_json::from_slice(calls[1].body.as_ref().unwrap()).unwrap();
    assert_eq!(body, json!({"reason": "stolen"}));
}

#[tokio::test]
async fn savings_routes_and_penalty_changed() {
    let (gb, f, _) = fake(
        vec![
            reply(
                200,
                json!({"account_id": "s1", "accrued_interest_minor": 5, "payouts": null}),
            ),
            reply(201, json!({"fixed_deposit": {"id": "f1"}})),
            api_err(409, "penalty_changed", json!({"quote": {}})),
        ],
        2,
    );
    assert_eq!(
        gb.accounts()
            .interest("s1")
            .await
            .unwrap()
            .accrued_interest_minor,
        5
    );
    let p = OpenFixedDepositParams {
        source_account_id: "a1".into(),
        product_id: "p1".into(),
        term_days: 30,
        amount_minor: 1000,
        ..Default::default()
    };
    gb.fixed_deposits().open(&p).await.unwrap();
    let e = gb
        .fixed_deposits()
        .break_deposit("f1", Some(1))
        .await
        .unwrap_err();
    assert!(e.kind() == ErrorKind::PenaltyChanged && e.is_conflict());
    let calls = f.calls();
    assert_eq!(
        calls.iter().map(route).collect::<Vec<_>>(),
        [
            "GET /v1/accounts/s1/interest",
            "POST /v1/fixed-deposits",
            "POST /v1/fixed-deposits/f1/break"
        ]
    );
    assert!(header(&calls[1], "idempotency-key").is_some());
    let body: Value = serde_json::from_slice(calls[2].body.as_ref().unwrap()).unwrap();
    assert_eq!(body, json!({"max_penalty_minor": 1}));
}

#[tokio::test]
async fn management_game_scopes_requests() {
    let f = Fake::default();
    f.0.lock().unwrap().replies = vec![reply(200, json!({"data": []}))].into();
    let m = GameBankManagement::builder()
        .api_key("gbe_dev_x")
        .base_url("http://engine.test")
        .transport(f.clone())
        .build()
        .unwrap();
    m.game("g1", "sandbox").banks().list().await.unwrap();
    let call = &f.calls()[0];
    assert_eq!(
        (header(call, "x-game-id"), header(call, "x-environment")),
        (Some("g1"), Some("sandbox"))
    );
}

#[test]
fn backoff() {
    for (attempt, lo, hi) in [
        (0, 0.375, 0.5),
        (1, 0.75, 1.0),
        (2, 1.5, 2.0),
        (10, 6.0, 8.0),
    ] {
        let d = retry_delay(attempt, None).as_secs_f64();
        assert!(d >= lo && d <= hi, "attempt {attempt}: {d}");
    }
    assert_eq!(retry_delay(0, Some(90.0)), Duration::from_secs(60));
    assert_eq!(retry_delay(3, Some(1.5)), Duration::from_millis(1500));
}
