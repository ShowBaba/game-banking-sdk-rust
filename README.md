# gamebanking (Rust)

[![crates.io](https://img.shields.io/crates/v/gamebanking.svg)](https://crates.io/crates/gamebanking)
[![docs.rs](https://docs.rs/gamebanking/badge.svg)](https://docs.rs/gamebanking)

Server-side SDK for the Game Banking Engine. Async (tokio + reqwest), Rust 1.88+.

> Game API keys move money. Use this SDK on your game servers only.
>
> In-game money only: the Game Banking Engine simulates banking for game currencies. It holds no real money and isn't a regulated financial service, so don't use it for balances players can cash out or exchange for real money.

## Install

From [crates.io](https://crates.io/crates/gamebanking):

```toml
[dependencies]
gamebanking = "0.5"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Or from Git:

```toml
gamebanking = { git = "https://github.com/ShowBaba/game-banking-sdk-rust" }
```

TLS uses rustls by default; use `default-features = false, features = ["native-tls"]` for the platform's TLS.

## Quick start

```rust
use gamebanking::{ErrorKind, FundingParams, GameBank, OpenAccountParams, TransferParams, UpsertPlayerParams};

let gb = GameBank::builder()
    .api_key(std::env::var("GBE_API_KEY")?)
    .build()?; // talks to https://api.gamebank.dev; or GameBank::from_env() with GBE_API_KEY

let player = gb.players().upsert(&UpsertPlayerParams {
    external_player_id: "user-42".into(),
    display_name: Some("Ada Obi".into()),
    ..Default::default()
}).await?;
let account = gb.accounts().open(&OpenAccountParams {
    bank_id: bank_id.clone(),
    player_id: Some(player.id),
    ..Default::default()
}).await?;

gb.funding()
    .idempotency_key("payroll-2040-06-01-user-42") // your stable ID: retrying never pays twice
    .create(&FundingParams {
        account_id: account.id.clone(),
        amount_minor: 500_000,
        reason: "salary".into(),
        reference: "payroll-2040-06-01-user-42".into(),
        ..Default::default()
    })
    .await?;

let sent = gb.transfers().create(&TransferParams {
    source_account_id: account.id.clone(),
    destination_account_number: Some("5915945174".into()),
    amount_minor: 1_000,
    ..Default::default()
}).await;
if let Err(e) = &sent {
    if e.kind() == ErrorKind::InsufficientFunds {
        println!("available {:?} (request {:?})", e.details.get("available_balance"), e.request_id);
    }
}

let mut lines = gb.accounts().iterate_transactions(&account.id, 50);
while let Some(line) = lines.next().await {
    let line = line?;
    println!("{} {}", line.direction, line.amount_display);
}
```

Each resource (`gb.transfers()`, `gb.cards()`, …) is a cheap handle; set per-call options on it before the call: `.idempotency_key(…)`, `.max_retries(…)`. Request parameters derive `Default`; unset `Option` fields are left out, so the engine's defaults apply.

Every failure is a `gamebanking::Error` with `status`, `code` (the engine's stable error code), `message`, `details`, `request_id` and `retry_after`. Match on `e.kind()` (`ErrorKind::NotFound`, `ErrorKind::FeeChanged`, …) or on `e.code`; `e.is_conflict()` and `e.is_unprocessable()` cover whole statuses.

## Fees and charges

```rust
use gamebanking::{fee_event, fee_model, CreateFeeRuleParams, QuoteFeesParams};

// Show the player what the transfer costs, then cap the fee at what they saw.
let q = gb.fees().quote(&QuoteFeesParams { bank_id: bank_id.clone(), amount_minor: 2_000_00, ..Default::default() }).await?;
let r = gb.transfers().create(&TransferParams { max_fee_minor: Some(q.fees_total_minor), ..transfer }).await;
// Err(e) with e.kind() == ErrorKind::FeeChanged: the schedule changed, quote again.

// Configure fees (bank:write): ₦50 + 1%, capped at ₦500, on transfers of ₦1,000 or more.
gb.fees().create(&bank_id, &CreateFeeRuleParams {
    code: "transfer_fee".into(), name: "Transfer fee".into(),
    event: fee_event::TRANSFER.into(), model: fee_model::COMBINED.into(),
    fixed_minor: Some(50_00), percentage_bps: Some(100),
    max_fee_minor: Some(500_00), min_amount_minor: Some(1_000_00),
    ..Default::default()
}).await?;
let books = gb.banks().books(&bank_id).await?; // fee_income_minor, levies_payable_minor, …
```

## Merchants and cards

```rust
use gamebanking::cards::*;

// A shop and its merchant account (card payments settle here).
let shop = gb.merchants().create(&CreateMerchantParams {
    external_merchant_id: "shop:mama-put".into(), name: "Mama Put".into(), category: Some("food".into()), ..Default::default()
}).await?;
let till = gb.accounts().open(&OpenAccountParams { bank_id: bank_id.clone(), merchant_id: Some(shop.id), ..Default::default() }).await?;

// Once per bank: turn on cards and add a program (bank:write).
gb.banks().update(&bank_id, &UpdateBankParams { feature_flags: Some([("cards".into(), true)].into()), ..Default::default() }).await?;
gb.card_programs().create(&bank_id, &CardProgramParams { name: Some("Classic".into()), ..Default::default() }).await?;

// Issue a card: card_number and cvv are shown only now.
let issued = gb.cards().issue(&IssueCardParams { account_id: account.id.clone(), ..Default::default() }).await?;

// Pay: an approved payment is charged at once (status "captured"). A decline is a result, not an error.
let paid = gb.card_authorizations().authorize(&AuthorizeCardParams {
    card_id: Some(issued.card.id.clone()), merchant_account_id: Some(till.id.clone()), amount_minor: 2_500_00, ..Default::default()
}).await?;
if let Some(code) = paid.authorization.declined() {
    show_player(code); // e.g. decline_code::INSUFFICIENT_FUNDS
}

// Or hold first when the final amount or the sale isn't settled yet (a fuel pump, a pre-order).
let held = gb.card_authorizations().authorize(&AuthorizeCardParams {
    card_id: Some(issued.card.id.clone()), merchant_account_id: Some(till.id.clone()), amount_minor: 2_500_00,
    capture: Some(false), ..Default::default()
}).await?;
// or release; refund later, in parts
gb.card_authorizations().capture(&held.authorization.id, &CaptureCardParams { amount_minor: Some(2_100_00), ..Default::default() }).await?;

gb.cards().report_lost(&issued.card.id, "stolen").await?; // final; no replacement is issued
```

## Savings and fixed deposits

```rust
use gamebanking::savings::*;

// Once per bank: turn savings on and define products (bank:write).
gb.banks().update(&bank_id, &UpdateBankParams { feature_flags: Some([("savings".into(), true)].into()), ..Default::default() }).await?;
let saver = gb.deposit_products().create(&bank_id, &DepositProductParams {
    code: Some("easy_saver".into()), name: Some("Easy Saver".into()), kind: Some(product_kind::SAVINGS.into()),
    rate_tiers: Some(vec![DepositTier { from_minor: 0, rate_bps: 300 }]),
    ..Default::default()
}).await?;
let term = gb.deposit_products().create(&bank_id, &DepositProductParams {
    code: Some("term".into()), name: Some("Term Deposit".into()), kind: Some(product_kind::FIXED_DEPOSIT.into()),
    terms: Some(vec![DepositTerm { days: 90, rate_bps: 800 }]),
    ..Default::default()
}).await?;

// A savings account earns daily on the game clock and is paid monthly.
let savings = gb.accounts().open(&OpenAccountParams {
    bank_id: bank_id.clone(), player_id: Some(player_id.clone()), product_id: Some(saver.id), ..Default::default()
}).await?;
let position = gb.accounts().interest(&savings.id).await?; // accrued_interest_minor, next_payout_day, payouts…

// A fixed deposit (deposits:write); break it early with a quote first.
let fd = gb.fixed_deposits().open(&OpenFixedDepositParams {
    source_account_id: account.id.clone(), product_id: term.id, term_days: 90, amount_minor: 50_000_00, ..Default::default()
}).await?;
let quote = gb.fixed_deposits().break_quote(&fd.fixed_deposit.id).await?;
gb.fixed_deposits().break_deposit(&fd.fixed_deposit.id, Some(quote.penalty_minor)).await?; // ErrorKind::PenaltyChanged if it rose
```

## Webhooks: debit and credit alerts

Every money movement on a player's account sends a signed `account.debited` or `account.credited` event to your endpoint. Verify it with the **raw** body. With axum, for example:

```rust
use axum::{body::Bytes, http::{HeaderMap, StatusCode}};
use gamebanking::webhooks::{event_type, SIGNATURE_HEADER};

async fn gbe_webhook(headers: HeaderMap, body: Bytes) -> StatusCode {
    let signature = headers.get(SIGNATURE_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    let Ok(event) = gamebanking::verify_webhook(&body, signature, &std::env::var("GBE_WEBHOOK_SECRET").unwrap()) else {
        return StatusCode::BAD_REQUEST; // e.reason says why
    };
    if event.r#type == event_type::ACCOUNT_DEBITED {
        if let Some(Ok(alert)) = event.alert() {
            // amount_minor, charges, total_minor, balance_after, counterparty, sequence…
            notify_player(alert.player_id, format!("-{}", alert.total_display));
        }
    }
    StatusCode::NO_CONTENT // answer within 10 s; delivery is at least once, so dedupe on event.id
}
```

Register the endpoint once (`webhooks:write`); the secret is shown only in the result:

```rust
use gamebanking::webhooks::CreateWebhookEndpointParams;
let ep = gb.webhooks().endpoints().create(&CreateWebhookEndpointParams {
    url: "https://game.example.com/gbe/webhooks".into(),
    event_types: Some(vec![event_type::ACCOUNT_DEBITED.into(), event_type::ACCOUNT_CREDITED.into()]),
    ..Default::default()
}).await?;
```

`gb.webhooks().endpoints()` also has `list`, `get`, `update` (status `disabled` / `active`), `delete`, `rotate_secret` (the old secret keeps signing for 24 hours; verify with both using `WebhookVerifier::new([new, old])`) and `test`. `gb.webhooks().events()` has `list` / `iterate` (the last 30 days, filterable by type and account, also a polling feed), `get`, `deliveries` and `redeliver`.

## Setup and CI

```rust
use gamebanking::{CreateBankParams, GameBankManagement, IssueApiKeyParams};

let mgmt = GameBankManagement::builder().api_key(std::env::var("GBE_DEV_CREDENTIAL")?).build()?;
let game = mgmt.games().create("Lagos Hustle", None).await?;
let key = mgmt.api_keys().issue(&game.id, &IssueApiKeyParams { environment: "sandbox".into(), ..Default::default() }).await?;
println!("{}", key.secret.unwrap()); // shown once
mgmt.game(&game.id, "sandbox").banks().create(&CreateBankParams {
    name: "Eko Trust Bank".into(), base_currency: "NGN".into(), ..Default::default()
}).await?;
```

`mgmt.games().update(…)` renames or suspends a game; `mgmt.games().delete(id)` deletes it and revokes its keys.

## Behaviour

Retries, idempotency keys and money helpers follow the shared contract in [`CONTRACT.md`](CONTRACT.md). Builder options: `timeout`, `max_retries`, `http_client` (your own `reqwest::Client`), `transport` (your own `HttpTransport`) and `on_response` (status, request ID and whether the engine replayed an idempotent response). `to_minor("100.50", 2)` and `format_minor(10050, 2)` convert amounts without floating point. The client never prints its credential, including in `Debug` output.

## Development

Unit tests need no network:

```bash
cargo test
```

The contract test runs the full journey against a real engine. `ci/docker-compose.yml` starts one (PostgreSQL plus the engine's published image) on port 8080:

```bash
docker compose -f ci/docker-compose.yml up -d && ./ci/wait-for-engine.sh
GBE_BASE_URL=http://localhost:8080 cargo test --test contract -- --nocapture
docker compose -f ci/docker-compose.yml down -v
```

The contract test's webhook receiver listens on any free port; set `GBE_WEBHOOK_RECEIVER_PORT` (and publish it) when the tests run in a container, and `GBE_WEBHOOK_RECEIVER_HOST` if the engine reaches your machine by another name than `host.docker.internal`. To test against a local engine build, build its image and set `GBE_ENGINE_IMAGE` to that tag. CI (`.github/workflows/ci.yml`) runs both on every push.
