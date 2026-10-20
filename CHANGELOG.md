# Changelog

## Unreleased

- **No base URL needed.** Clients talk to the hosted engine at `https://api.gamebank.dev` by default (`DEFAULT_BASE_URL`). `.base_url(...)` and `GBE_BASE_URL` still override it, for a local or self-hosted engine. The `missing_base_url` error is gone. `GameBank` and `GameBankManagement` have `base_url()`.

## 0.5.0

First release of the Rust SDK, at feature parity with the other SDKs' 0.5.0:

- **Clients.** `GameBank` (game API key) and `GameBankManagement` (developer credential or session token), built with `builder()` or `from_env()`. Async, on tokio and reqwest (rustls by default, `native-tls` feature available); a pluggable `HttpTransport`.
- **Runtime.** Players, accounts (with paged history and `iterate_transactions`), funding, transfers, holds, reversals, name enquiry, banks, limits, books, currencies, the game clock and reconciliation.
- **Fees and charges.** Fee rules, quotes, and `max_fee_minor` on transfers, holds and captures (`ErrorKind::FeeChanged`).
- **Webhooks.** `verify_webhook` and `WebhookVerifier` (several secrets, tolerance) check the `GBE-Signature` header; endpoints and the event log (with an auto-paging `iterate`); typed `AccountAlert` data.
- **Merchants and cards.** Merchants, card programs, cards (issue, block, unblock, report lost, close) and card authorisations (an approved payment is captured in the same call unless `capture: Some(false)` holds it for a later capture or release; release; refund). Declines are results with a `decline_code`.
- **Savings.** Deposit products and quotes, savings accounts and `accounts().interest`, fixed deposits (open, update, break quote, `break_deposit`; `ErrorKind::PenaltyChanged`).
- **Management.** Games, API keys, developer credentials, audit logs, and `game(id, environment)` to configure a game with a developer credential.
- Idempotency keys generated once per call and reused on retries; retries with backoff and `Retry-After`; a response hook; `to_minor` and `format_minor`.
