//! The PRD §16.2 journey against a real engine. Runs when GBE_BASE_URL is set.
//!
//! The webhook part runs a local receiver that the engine reaches at
//! GBE_WEBHOOK_RECEIVER_HOST (default host.docker.internal) on
//! GBE_WEBHOOK_RECEIVER_PORT (default: any free port; publish a fixed one when
//! the tests run in a container).

// Amounts are written naira_kobo, e.g. 2_500_00 is ₦2,500.00.
#![allow(clippy::inconsistent_digit_grouping)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gamebanking::cards::{
    card_status, decline_code, AuthorizeCardParams, CaptureCardParams, CardProgramParams,
    CreateMerchantParams, IssueCardParams, ListCardAuthorizationsParams, ListCardsParams,
    RefundCardParams,
};
use gamebanking::savings::{
    product_kind, DepositProductParams, DepositQuoteParams, DepositTerm, DepositTier,
    OpenFixedDepositParams,
};
use gamebanking::webhooks::{
    event_type, CreateWebhookEndpointParams, ListWebhookEventsParams, UpdateWebhookEndpointParams,
    SIGNATURE_HEADER,
};
use gamebanking::*;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// Forwards requests but reports the first POST /v1/transfers as a network
/// failure after the engine has processed it.
struct Lossy {
    inner: ReqwestTransport,
    dropped: Arc<AtomicBool>,
}

impl HttpTransport for Lossy {
    fn send(
        &self,
        req: HttpRequest,
    ) -> BoxFuture<'_, std::result::Result<HttpResponse, TransportError>> {
        Box::pin(async move {
            let transfer =
                req.method == reqwest::Method::POST && req.url.ends_with("/v1/transfers");
            let res = self.inner.send(req).await;
            if res.is_ok() && transfer && !self.dropped.swap(true, Ordering::SeqCst) {
                return Err(TransportError::Connection(
                    "connection reset after the engine processed the request".into(),
                ));
            }
            res
        })
    }
}

/// One webhook request the receiver got.
#[derive(Clone, Debug)]
struct Delivery {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Delivery {
    fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
}

/// A local webhook endpoint that keeps each raw delivery.
struct Receiver {
    url: String,
    rx: mpsc::UnboundedReceiver<Delivery>,
    all: Vec<Delivery>,
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Reads HTTP/1.1 requests off one connection and answers each with 204.
async fn serve(mut conn: TcpStream, tx: mpsc::UnboundedSender<Delivery>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let end = loop {
            if let Some(i) = find(&buf, b"\r\n\r\n") {
                break i;
            }
            match conn.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..end]).to_string();
        let headers: Vec<(String, String)> = head
            .lines()
            .skip(1)
            .filter_map(|l| {
                l.split_once(':')
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        let len: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        while buf.len() < end + 4 + len {
            match conn.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let body = buf[end + 4..end + 4 + len].to_vec();
        buf.drain(..end + 4 + len);
        let _ = tx.send(Delivery { headers, body });
        if conn
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn start_receiver() -> Receiver {
    let port: u16 = std::env::var("GBE_WEBHOOK_RECEIVER_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let host = std::env::var("GBE_WEBHOOK_RECEIVER_HOST")
        .unwrap_or_else(|_| "host.docker.internal".into());
    let ln = TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind the webhook receiver");
    let url = format!("http://{host}:{}/hooks", ln.local_addr().unwrap().port());
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((conn, _)) = ln.accept().await {
            tokio::spawn(serve(conn, tx.clone()));
        }
    });
    Receiver {
        url,
        rx,
        all: Vec::new(),
    }
}

impl Receiver {
    /// The first delivery (old or, with `fresh`, only new) whose body contains `s`.
    async fn wait_for(&mut self, s: &str, fresh: bool) -> Delivery {
        let has = |d: &Delivery| find(&d.body, s.as_bytes()).is_some();
        if !fresh {
            if let Some(d) = self.all.iter().find(|d| has(d)) {
                return d.clone();
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let d = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("no webhook delivery containing {s:?} within 20s"))
                .expect("receiver stopped");
            self.all.push(d.clone());
            if has(&d) {
                return d;
            }
        }
    }
}

fn uniq() -> String {
    format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn contract() {
    let Ok(base) = std::env::var("GBE_BASE_URL") else {
        eprintln!("skipped: set GBE_BASE_URL to run against an engine");
        return;
    };

    // Sign up (a browser flow, so a plain request) for a developer session token.
    let reg: serde_json::Value = {
        let res = reqwest::Client::new()
            .post(format!("{base}/v1/auth/register"))
            .header("Content-Type", "application/json")
            .body(
                json!({"name": "SDK Contract RUST", "email": format!("sdk-rust-{}@studio.test", uniq()), "password": "contract-test-password"})
                    .to_string(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 201, "register");
        serde_json::from_slice(&res.bytes().await.unwrap()).unwrap()
    };
    let token = reg["session"]["token"].as_str().unwrap();

    let mgmt = GameBankManagement::builder()
        .api_key(token)
        .base_url(&base)
        .build()
        .unwrap();
    assert_eq!(mgmt.me().await.unwrap().name, "SDK Contract RUST");
    let game = mgmt
        .games()
        .create(&format!("RUST Game {}", uniq()), None)
        .await
        .unwrap();
    let key = mgmt
        .api_keys()
        .issue(
            &game.id,
            &IssueApiKeyParams {
                environment: "sandbox".into(),
                label: s("contract"),
                scopes: Some(scope::ALL.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let secret = key.secret.clone().unwrap();
    assert!(secret.starts_with("gbe_test_sk_"), "secret {secret}");
    let cfg = mgmt.game(&game.id, "sandbox");
    let bank = cfg
        .banks()
        .create(&CreateBankParams {
            name: "Eko Trust Bank".into(),
            base_currency: "NGN".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        bank.system_accounts.len(),
        8,
        "system accounts incl. levies_payable"
    );
    let read_only = mgmt
        .api_keys()
        .issue(
            &game.id,
            &IssueApiKeyParams {
                environment: "sandbox".into(),
                scopes: Some(vec![scope::ACCOUNT_READ.into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let seen: Arc<Mutex<Vec<ResponseInfo>>> = Arc::default();
    let hook = seen.clone();
    let gb = GameBank::builder()
        .api_key(&secret)
        .base_url(&base)
        .on_response(move |r| hook.lock().unwrap().push(r.clone()))
        .build()
        .unwrap();
    assert_eq!(gb.environment(), Some(Environment::Sandbox));

    let ada = gb
        .players()
        .upsert(&UpsertPlayerParams {
            external_player_id: format!("ada-{}", uniq()),
            display_name: s("Ada Obi"),
            ..Default::default()
        })
        .await
        .unwrap();
    let again = gb
        .players()
        .upsert(&UpsertPlayerParams {
            external_player_id: ada.external_player_id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(again.id, ada.id, "upsert is idempotent");
    let tunde = gb
        .players()
        .upsert(&UpsertPlayerParams {
            external_player_id: format!("tunde-{}", uniq()),
            display_name: s("Tunde Bello"),
            ..Default::default()
        })
        .await
        .unwrap();
    let open = |player_id: &str| OpenAccountParams {
        bank_id: bank.id.clone(),
        player_id: s(player_id),
        ..Default::default()
    };
    let ada_acct = gb.accounts().open(&open(&ada.id)).await.unwrap();
    let tunde_acct = gb.accounts().open(&open(&tunde.id)).await.unwrap();

    let reference = format!("pay-{}", uniq());
    let salary = FundingParams {
        account_id: ada_acct.id.clone(),
        amount_minor: 10_000_00,
        reason: "salary".into(),
        reference: reference.clone(),
        ..Default::default()
    };
    let fund_key = format!("fund-{reference}");
    let funded = gb
        .funding()
        .idempotency_key(&fund_key)
        .create(&salary)
        .await
        .unwrap();
    assert_eq!(funded.balance.posted_balance, 10_000_00);
    let replay = gb
        .funding()
        .idempotency_key(&fund_key)
        .create(&salary)
        .await
        .unwrap();
    assert_eq!(replay.transaction.id, funded.transaction.id);
    assert!(
        seen.lock().unwrap().last().unwrap().replayed,
        "replay not flagged"
    );
    let changed = FundingParams {
        amount_minor: 1,
        ..salary.clone()
    };
    let e = gb
        .funding()
        .idempotency_key(&fund_key)
        .create(&changed)
        .await
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Conflict, "key reuse: {e}");

    let who = gb
        .name_enquiry(&bank.bank_code, &tunde_acct.account_number)
        .await
        .unwrap();
    assert_eq!(who.account_name, "Tunde Bello");
    let tr = gb
        .transfers()
        .create(&TransferParams {
            source_account_id: ada_acct.id.clone(),
            destination_account_number: Some(tunde_acct.account_number.clone()),
            amount_minor: 2_500_00,
            narration: s("Generator"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(tr.source_balance.posted_balance, 7_500_00);
    assert_eq!(
        tr.destination_balance.as_ref().unwrap().posted_balance,
        2_500_00
    );

    let pay = |from: &str, to: &str, amount: i64| TransferParams {
        source_account_id: from.into(),
        destination_account_id: s(to),
        amount_minor: amount,
        ..Default::default()
    };
    let e = gb
        .transfers()
        .create(&pay(&tunde_acct.id, &ada_acct.id, 99_999_00))
        .await
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InsufficientFunds, "{e}");
    assert_eq!(e.details["available_balance"], json!(2_500_00));
    assert!(e.request_id.is_some());
    let e = gb
        .accounts()
        .get("00000000-0000-0000-0000-000000000000")
        .await
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    let ro = GameBank::builder()
        .api_key(read_only.secret.unwrap())
        .base_url(&base)
        .build()
        .unwrap();
    let e = ro
        .transfers()
        .create(&pay(&ada_acct.id, &tunde_acct.id, 1))
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind(), e.code.as_str()),
        (ErrorKind::PermissionDenied, "insufficient_scope")
    );

    let held = gb
        .holds()
        .place(&PlaceHoldParams {
            account_id: ada_acct.id.clone(),
            amount_minor: 3_000_00,
            reason: "Order #1".into(),
            destination_account_id: s(&tunde_acct.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        (
            held.balance.posted_balance,
            held.balance.available_balance,
            held.hold.reserved_fee_minor
        ),
        (7_500_00, 4_500_00, 0)
    );
    let capt = gb
        .holds()
        .capture(
            &held.hold.id,
            &CaptureHoldParams {
                amount_minor: Some(2_000_00),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (
            capt.hold.status.as_str(),
            capt.source_balance.available_balance
        ),
        ("captured", 5_500_00)
    );

    let dropped = Arc::new(AtomicBool::new(false));
    let lossy = GameBank::builder()
        .api_key(&secret)
        .base_url(&base)
        .transport(Lossy {
            inner: ReqwestTransport::default(),
            dropped: dropped.clone(),
        })
        .build()
        .unwrap();
    let lost = lossy
        .transfers()
        .create(&pay(&ada_acct.id, &tunde_acct.id, 500_00))
        .await
        .unwrap();
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(lost.source_balance.posted_balance, 5_000_00);
    assert_eq!(
        gb.accounts()
            .balance(&ada_acct.id)
            .await
            .unwrap()
            .posted_balance,
        5_000_00,
        "posted twice"
    );

    let history = gb
        .accounts()
        .iterate_transactions(&ada_acct.id, 2)
        .collect()
        .await
        .unwrap();
    assert_eq!(
        history.iter().map(|i| i.amount_minor).collect::<Vec<_>>(),
        [500_00, 2_000_00, 2_500_00, 10_000_00]
    );

    let rev = gb
        .transactions()
        .reverse(
            &tr.transaction.id,
            &ReverseParams {
                reason: "customer dispute".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let orig = gb.transactions().get(&tr.transaction.id).await.unwrap();
    assert_eq!(
        (rev.r#type.as_str(), orig.status.as_str()),
        ("reversal", "reversed")
    );

    // Fees: a ₦50 + 1% transfer fee (max ₦500) via the developer credential, a levy via the game key.
    let fee = cfg
        .fees()
        .create(
            &bank.id,
            &CreateFeeRuleParams {
                code: "transfer_fee".into(),
                name: "Transfer fee".into(),
                event: fee_event::TRANSFER.into(),
                model: fee_model::COMBINED.into(),
                fixed_minor: Some(50_00),
                percentage_bps: Some(100),
                max_fee_minor: Some(500_00),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (fee.state.as_str(), fee.credit_account.as_str()),
        (fee_state::IN_EFFECT, "fee_income")
    );
    let levy = gb
        .fees()
        .create(
            &bank.id,
            &CreateFeeRuleParams {
                code: "stamp_duty".into(),
                name: "Stamp duty".into(),
                event: fee_event::TRANSFER.into(),
                kind: s(fee_kind::LEVY),
                model: fee_model::FIXED.into(),
                fixed_minor: Some(50_00),
                min_amount_minor: Some(10_000_00),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(levy.credit_account, "levies_payable");
    assert_eq!(gb.fees().list(&bank.id).await.unwrap().len(), 2);
    let quote = gb
        .fees()
        .quote(&QuoteFeesParams {
            bank_id: bank.id.clone(),
            amount_minor: 1_000_00,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(quote.fees_total_minor, 60_00);
    let before = gb.accounts().balance(&ada_acct.id).await.unwrap();
    let capped = TransferParams {
        max_fee_minor: Some(quote.fees_total_minor),
        ..pay(&ada_acct.id, &tunde_acct.id, 1_000_00)
    };
    let paid = gb.transfers().create(&capped).await.unwrap();
    assert_eq!(
        (paid.fees_total_minor, paid.total_debit_minor),
        (60_00, 1_060_00)
    );
    assert_eq!(
        paid.source_balance.posted_balance,
        before.posted_balance - 1_060_00
    );
    let count = |kind: &str| {
        paid.transaction
            .entries
            .iter()
            .filter(|e| e.kind == kind)
            .count()
    };
    assert_eq!(
        (count(entry_kind::PRINCIPAL), count(entry_kind::FEE)),
        (2, 2)
    );
    let latest = gb
        .accounts()
        .transactions(&ada_acct.id, Some(2), None)
        .await
        .unwrap();
    assert!(
        latest
            .data
            .iter()
            .any(|l| l.kind == "fee" && l.narration.as_deref() == Some("Transfer fee")),
        "the fee should be its own labelled statement line"
    );
    cfg.fees()
        .update(
            &bank.id,
            &fee.id,
            &UpdateFeeRuleParams {
                fixed_minor: Some(60_00),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let e = gb.transfers().create(&capped).await.unwrap_err();
    assert!(
        e.kind() == ErrorKind::FeeChanged && e.is_conflict(),
        "fee_changed: {e}"
    );
    assert_eq!(
        gb.banks().books(&bank.id).await.unwrap().fee_income_minor,
        60_00
    );
    gb.transactions()
        .reverse(
            &paid.transaction.id,
            &ReverseParams {
                reason: "refund".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        gb.banks().books(&bank.id).await.unwrap().fee_income_minor,
        0,
        "fee not refunded"
    );
    let paused = cfg
        .fees()
        .update(
            &bank.id,
            &fee.id,
            &UpdateFeeRuleParams {
                status: s("inactive"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(paused.state, fee_state::INACTIVE);

    // Webhooks: a debit alert reaches a real endpoint, signed, and verifies with the SDK helper.
    assert!(cfg.webhooks().endpoints().list().await.unwrap().is_empty());
    assert_eq!(
        ro.webhooks().endpoints().list().await.unwrap_err().kind(),
        ErrorKind::PermissionDenied
    );
    let mut rc = start_receiver().await;
    let ep = gb
        .webhooks()
        .endpoints()
        .create(&CreateWebhookEndpointParams {
            url: rc.url.clone(),
            event_types: Some(vec![event_type::ACCOUNT_DEBITED.into()]),
            description: s("contract"),
        })
        .await
        .unwrap();
    let ep_secret = ep.secret.clone().unwrap();
    assert!(!ep.secret_prefix.is_empty() && ep_secret.starts_with(&ep.secret_prefix));
    assert!(
        gb.webhooks()
            .endpoints()
            .get(&ep.id)
            .await
            .unwrap()
            .secret
            .is_none(),
        "the secret is shown once"
    );
    let probe = gb.webhooks().endpoints().test(&ep.id).await.unwrap();
    assert_eq!(
        probe.state, "delivered",
        "test send: {:?}",
        probe.last_error
    );
    let sent = gb
        .transfers()
        .create(&TransferParams {
            narration: s("Webhook check"),
            ..pay(&ada_acct.id, &tunde_acct.id, 5_00)
        })
        .await
        .unwrap();
    let hit = rc.wait_for(&sent.transaction.id, false).await;
    let event = verify_webhook(&hit.body, hit.header(SIGNATURE_HEADER), &ep_secret).unwrap();
    let alert = event.alert().unwrap().unwrap();
    assert_eq!(event.r#type, event_type::ACCOUNT_DEBITED);
    assert_eq!(
        (alert.account_id.as_str(), alert.total_minor),
        (ada_acct.id.as_str(), 5_00)
    );
    assert_eq!(
        alert.counterparty.as_ref().unwrap().account_name,
        "Tunde Bello"
    );
    assert_eq!(
        alert.balance_after.posted_minor,
        sent.source_balance.posted_balance
    );
    assert_eq!(hit.header("GBE-Event-Id"), event.id);
    let feed = gb
        .webhooks()
        .events()
        .list(&ListWebhookEventsParams {
            r#type: s(event_type::ACCOUNT_DEBITED),
            account_id: s(&ada_acct.id),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(feed.data.len(), 1);
    assert_eq!(feed.data[0].id, event.id);
    assert_eq!(
        gb.webhooks().events().get(&event.id).await.unwrap().id,
        event.id
    );
    let ds = gb.webhooks().events().deliveries(&event.id).await.unwrap();
    assert!(
        ds.len() == 1 && ds[0].state == "delivered",
        "deliveries {ds:?}"
    );
    let rotated = gb
        .webhooks()
        .endpoints()
        .rotate_secret(&ep.id)
        .await
        .unwrap();
    let new_secret = rotated.secret.clone().unwrap();
    assert!(new_secret != ep_secret && rotated.previous_secret_expires_at.is_some());
    gb.webhooks()
        .events()
        .redeliver(&event.id, Some(&ep.id))
        .await
        .unwrap();
    let redelivered = rc.wait_for(&sent.transaction.id, true).await;
    let header = redelivered.header(SIGNATURE_HEADER).to_string();
    WebhookVerifier::new([new_secret])
        .verify(&redelivered.body, &header)
        .expect("new secret");
    verify_webhook(&redelivered.body, &header, &ep_secret).expect("old secret during the overlap");
    let up = gb
        .webhooks()
        .endpoints()
        .update(
            &ep.id,
            &UpdateWebhookEndpointParams {
                status: s("disabled"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(up.status, "disabled");
    gb.webhooks().endpoints().delete(&ep.id).await.unwrap();
    assert_eq!(
        gb.webhooks()
            .endpoints()
            .get(&ep.id)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );

    // Merchants and cards: a shop with a merchant account, a card, a payment, a refund, a lost card.
    let shop = gb
        .merchants()
        .create(&CreateMerchantParams {
            external_merchant_id: format!("shop-{}", uniq()),
            name: "Mama Put".into(),
            category: s("food"),
            ..Default::default()
        })
        .await
        .unwrap();
    let same = gb
        .merchants()
        .create(&CreateMerchantParams {
            external_merchant_id: shop.external_merchant_id.clone(),
            name: "x".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(same.id, shop.id, "merchants are idempotent");
    let till = gb
        .accounts()
        .open(&OpenAccountParams {
            bank_id: bank.id.clone(),
            merchant_id: s(&shop.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        (till.owner_type.as_str(), till.merchant_id.as_deref()),
        ("merchant", Some(shop.id.as_str()))
    );
    let flags = |flag: &str| UpdateBankParams {
        feature_flags: Some(BTreeMap::from([(flag.to_string(), true)])),
        ..Default::default()
    };
    cfg.banks().update(&bank.id, &flags("cards")).await.unwrap();
    let program = cfg
        .card_programs()
        .create(
            &bank.id,
            &CardProgramParams {
                name: s("Classic"),
                daily_limit_minor: Some(3_000_00),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(program.bin.len(), 6);
    let issued = gb
        .cards()
        .issue(&IssueCardParams {
            account_id: ada_acct.id.clone(),
            name_on_card: s("Ada Obi"),
            ..Default::default()
        })
        .await
        .unwrap();
    let pan = issued.card_number.clone();
    assert_eq!(pan.len(), 16);
    assert_eq!(
        issued.card.masked_number,
        format!("{}******{}", &pan[..6], &pan[12..])
    );
    let spendable = gb
        .accounts()
        .balance(&ada_acct.id)
        .await
        .unwrap()
        .available_balance;
    let auth = gb
        .card_authorizations()
        .authorize(&AuthorizeCardParams {
            card_number: s(&pan),
            expiry_month: Some(issued.card.expiry_month),
            expiry_year: Some(issued.card.expiry_year),
            cvv: s(&issued.cvv),
            merchant_account_number: s(&till.account_number),
            amount_minor: 200_00,
            capture: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        (
            auth.authorization.status.as_str(),
            auth.balance.available_balance
        ),
        ("approved", spendable - 200_00)
    );
    let captured = gb
        .card_authorizations()
        .capture(
            &auth.authorization.id,
            &CaptureCardParams {
                amount_minor: Some(150_00),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(captured.transaction.r#type, "card_payment");
    assert_eq!(
        (
            captured.merchant_balance.posted_balance,
            captured.card_balance.available_balance
        ),
        (150_00, spendable - 150_00)
    );
    let refunded = gb
        .card_authorizations()
        .refund(
            &auth.authorization.id,
            &RefundCardParams {
                amount_minor: Some(50_00),
                reason: s("cold"),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (
            refunded.transaction.r#type.as_str(),
            refunded.authorization.refunded_minor
        ),
        ("card_refund", 50_00)
    );
    let instant = gb
        .card_authorizations()
        .authorize(&AuthorizeCardParams {
            card_id: s(&issued.card.id),
            merchant_account_id: s(&till.id),
            amount_minor: 20_00,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(instant.authorization.status, "captured");
    assert_eq!(instant.transaction.as_ref().unwrap().r#type, "card_payment");
    assert_eq!(
        instant.merchant_balance.as_ref().unwrap().posted_balance,
        120_00
    );
    let by_id = AuthorizeCardParams {
        card_id: s(&issued.card.id),
        merchant_account_id: s(&till.id),
        amount_minor: 10_00,
        capture: Some(false),
        ..Default::default()
    };
    let pending = gb.card_authorizations().authorize(&by_id).await.unwrap();
    assert_eq!(
        gb.cards()
            .report_lost(&issued.card.id, "stolen")
            .await
            .unwrap()
            .status,
        card_status::STOLEN
    );
    let refused = gb.card_authorizations().authorize(&by_id).await.unwrap();
    assert_eq!(
        refused.authorization.declined(),
        Some(decline_code::CARD_STOLEN)
    );
    let late = gb
        .card_authorizations()
        .capture(&pending.authorization.id, &CaptureCardParams::default())
        .await
        .unwrap();
    assert_eq!(
        late.authorization.status, "captured",
        "approved payments still capture after a loss"
    );
    let cards = gb
        .cards()
        .list(&ListCardsParams {
            account_id: s(&ada_acct.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(cards.len(), 1, "no replacement card");
    let auths = gb
        .card_authorizations()
        .list(&ListCardAuthorizationsParams {
            card_id: s(&issued.card.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(auths.len(), 4);

    gb.clock()
        .configure(&ConfigureClockParams {
            mode: s("manual"),
            ..Default::default()
        })
        .await
        .unwrap();
    let adv = gb
        .clock()
        .advance(&AdvanceClockParams {
            days: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!adv.clock.game_day.is_empty());
    let recon = gb.reconcile().await.unwrap();
    assert!(recon.ok, "reconciliation: {:?}", recon.differences);

    // Savings: ₦100,000 at 36.5% earns ₦100 a game day; a fixed deposit can be broken early.
    cfg.banks()
        .update(&bank.id, &flags("savings"))
        .await
        .unwrap();
    let saver = cfg
        .deposit_products()
        .create(
            &bank.id,
            &DepositProductParams {
                code: s("easy_saver"),
                name: s("Easy Saver"),
                kind: s(product_kind::SAVINGS),
                rate_tiers: Some(vec![DepositTier {
                    from_minor: 0,
                    rate_bps: 3650,
                }]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let term = cfg
        .deposit_products()
        .create(
            &bank.id,
            &DepositProductParams {
                code: s("term"),
                name: s("Term Deposit"),
                kind: s(product_kind::FIXED_DEPOSIT),
                terms: Some(vec![DepositTerm {
                    days: 30,
                    rate_bps: 3650,
                }]),
                early_forfeit_bps: Some(10_000),
                early_penalty_fixed_minor: Some(100_00),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let savings = gb
        .accounts()
        .open(&OpenAccountParams {
            bank_id: bank.id.clone(),
            player_id: s(&ada.id),
            product_id: s(&saver.id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(savings.account_type, "savings");
    let fund = |account_id: &str, reference: String| FundingParams {
        account_id: account_id.into(),
        amount_minor: 100_000_00,
        reason: "savings".into(),
        reference,
        ..Default::default()
    };
    gb.funding()
        .create(&fund(&savings.id, format!("save-{}", uniq())))
        .await
        .unwrap();
    gb.funding()
        .create(&fund(&ada_acct.id, format!("fd-{}", uniq())))
        .await
        .unwrap();
    let fd = gb
        .fixed_deposits()
        .open(&OpenFixedDepositParams {
            source_account_id: ada_acct.id.clone(),
            product_id: term.id.clone(),
            term_days: 30,
            amount_minor: 100_000_00,
            ..Default::default()
        })
        .await
        .unwrap();
    let q = gb
        .deposit_products()
        .quote(&DepositQuoteParams {
            product_id: term.id.clone(),
            amount_minor: 100_000_00,
            term_days: Some(30),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(q.gross_interest_minor, 3_000_00);
    gb.clock()
        .advance(&AdvanceClockParams {
            days: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        gb.accounts()
            .interest(&savings.id)
            .await
            .unwrap()
            .accrued_interest_minor,
        200_00
    );
    let bq = gb
        .fixed_deposits()
        .break_quote(&fd.fixed_deposit.id)
        .await
        .unwrap();
    assert_eq!(bq.payout_minor, 100_000_00 - 100_00);
    let e = gb
        .fixed_deposits()
        .break_deposit(&fd.fixed_deposit.id, Some(50_00))
        .await
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::PenaltyChanged, "penalty cap: {e}");
    let broken = gb
        .fixed_deposits()
        .break_deposit(&fd.fixed_deposit.id, Some(bq.penalty_minor))
        .await
        .unwrap();
    assert_eq!(broken.fixed_deposit.status, "broken");

    // Renaming and deleting a game. Deleting revokes its keys at once.
    let renamed = mgmt
        .games()
        .update(
            &game.id,
            &UpdateGameParams {
                name: s("RUST Game renamed"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "RUST Game renamed");
    mgmt.games().delete(&game.id).await.unwrap();
    assert_eq!(
        mgmt.games().get(&game.id).await.unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        gb.clock().get().await.unwrap_err().kind(),
        ErrorKind::Authentication,
        "deleted game's key"
    );
}
