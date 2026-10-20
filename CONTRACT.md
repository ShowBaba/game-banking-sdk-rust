# SDK contract

The behaviour every Game Banking Engine SDK implements. The canonical copy lives in the engine repository (`docs/sdk-contract.md`); each SDK repository carries a copy as `CONTRACT.md`. Change the engine's copy first, then update the SDKs.

| SDK | Repository | Package | Install |
| --- | --- | --- | --- |
| TypeScript / JavaScript | [game-banking-sdk-typescript](https://github.com/ShowBaba/game-banking-sdk-typescript) | [`@gamebanking/sdk`](https://www.npmjs.com/package/@gamebanking/sdk) on npm | `npm i @gamebanking/sdk` |
| Python | [game-banking-sdk-python](https://github.com/ShowBaba/game-banking-sdk-python) | [`game-banking-sdk`](https://pypi.org/project/game-banking-sdk/) on PyPI (imported as `gamebanking`) | `pip install game-banking-sdk` |
| C# / .NET | [game-banking-sdk-dotnet](https://github.com/ShowBaba/game-banking-sdk-dotnet) | [`GameBanking`](https://www.nuget.org/packages/GameBanking/) on NuGet | `dotnet add package GameBanking` |
| Go | [game-banking-sdk-go](https://github.com/ShowBaba/game-banking-sdk-go) | `github.com/ShowBaba/game-banking-sdk-go` | `go get github.com/ShowBaba/game-banking-sdk-go` |
| Rust | [game-banking-sdk-rust](https://github.com/ShowBaba/game-banking-sdk-rust) | [`gamebanking`](https://crates.io/crates/gamebanking) on crates.io | `cargo add gamebanking` |

**The engine handles in-game money only.** It simulates banking for game currencies, holds no real funds and isn't a regulated financial service. SDK documentation says so.

**The SDKs are for game servers.** A game API key can move money, so it must never ship in a game client or a web page. The SDKs never authenticate players: your server verifies its own player, then calls the engine for them.

## Two clients

| Client | Credential | Use it for |
| --- | --- | --- |
| `GameBank` | Game API key (`gbe_test_sk_…` sandbox, `gbe_live_sk_…` production) | Everything your game server does at runtime: players, accounts, funding, transfers, holds, history, name enquiry, the clock, bank configuration. |
| `GameBankManagement` | Developer credential (`gbe_dev_…`) or a sign-in session token | Setup and CI: games, API keys, developer credentials, audit logs. `management.game(gameId, environment)` returns a `GameBank` that configures that game's banks, currencies, limits and clock with your developer credential. |

## Behaviour every SDK implements

This is the contract. Each SDK's tests check it.

### Configuration
`apiKey` (or the `GBE_API_KEY` environment variable), `baseUrl` (optional: the explicit option, else `GBE_BASE_URL`, else the hosted engine `https://api.gamebank.dev`, exported as a `DEFAULT_BASE_URL` constant; set it only for a local or self-hosted engine), `timeout` (30 s) and `maxRetries` (2). The environment is read from the key prefix and exposed as `environment`; it is never sent, because the engine derives it from the key.

### Idempotency
Every money-moving call (`funding.create`, `transfers.create`, `holds.place`, `holds.capture`, `holds.release`, `transactions.reverse`) sends an `Idempotency-Key`.

* Pass your own key when you have a stable ID for the operation (an order ID, a payroll run ID). Retrying with the same key never moves money twice.
* If you don't pass one, the SDK generates a random key **once per call** and reuses it on its own retries. That protects against network failures inside one call, but not against your code calling twice; for that, pass your own key.

### Retries
Up to `maxRetries` extra attempts, with exponential backoff (0.5 s, 1 s, 2 s … capped at 8 s, with jitter). `Retry-After` is honoured, up to 60 s.

| Failure | Retried when |
| --- | --- |
| `429 rate_limited` | Always (the request was not processed). |
| Network error or timeout, `500`, `502`, `503`, `504`, `409 request_in_progress` | Only if retrying cannot duplicate an effect: `GET` and `PUT` requests, requests that carry an `Idempotency-Key`, and `players.upsert`. A plain `POST` such as `accounts.open` is never retried after it may have reached the server. |
| Any other `4xx` | Never. |

### Errors
Every failure raises (or returns, in Go and Rust) a `GameBankError` with `status`, `code` (the engine's stable error code), `message`, `details`, `requestId` (the engine's correlation ID) and `retryAfter`. Subtypes let you catch categories:

| Type | When |
| --- | --- |
| `AuthenticationError` | 401 |
| `PermissionDeniedError` | 403 (missing scope, suspended game, …) |
| `NotFoundError` | 404 |
| `ConflictError` | 409 (`idempotency_key_reused`, `email_taken`, `already_reversed`, …) |
| `FeeChangedError` (a `ConflictError`) | 409 `fee_changed`: the fees now total more than the request's `max_fee_minor`; `details.fees` lists the current charges |
| `InvalidRequestError` | 400, 413, 415 |
| `InsufficientFundsError` | 422 `insufficient_funds` |
| `LimitExceededError` | 422 `limit_exceeded` |
| `UnprocessableError` | Any other 422 business rule (`account_frozen`, `hold_not_active`, …) |
| `RateLimitError` | 429 |
| `ServerError` | 5xx |
| `ConnectionError` | No response: network failure or timeout (`status` 0) |

Branch on `code` for anything finer; it is stable across versions.

### Money
Amounts are integers in minor units (`amount_minor`); a currency's `exponent` gives its decimal places. Each SDK has `toMinor("100.50", 2) → 10050` and `formatMinor(10050, 2) → "100.50"` helpers that never use floating point. The TypeScript SDK rejects amounts that are not safe integers.

### Fees and charges

A bank's fee rules (`fees.list/get/create/update`, needing `bank:write` to change) add their own ledger lines to transfers and hold captures, and a monthly maintenance charge. Every SDK supports the full flow:

1. `fees.quote({ bank_id, amount_minor })` returns the exact charges for a payment now.
2. Show the player the total, then pass the quote's `fees_total_minor` as `max_fee_minor` on `transfers.create` (or `holds.place` / `holds.capture`). If the schedule changed in between and the fees are now higher, the engine refuses with `FeeChangedError` rather than charging more; get a new quote.
3. Results carry `fees`, `fees_total_minor` and `total_debit_minor`; ledger entries and history lines carry `kind` (`principal`, `fee` or `levy`), and a fee line's narration is the fee's name.

Fee rules report a `state` (`in_effect`, `scheduled`, `ended`, `inactive`) computed from the game clock; `start_now` starts a rule at the current game time. Holds carry `reserved_fee_minor`. `banks.books(bank_id)` returns the bank's fee income and levies held. A reversal undoes the whole transaction and refunds its fees unless the bank turned that off.

### Webhooks

Every SDK ships a verification helper that receivers call with the **raw** body,
the `GBE-Signature` header and the endpoint secret, plus an optional tolerance
(default 5 minutes). It parses `t` and every `v1` value, and rejects a timestamp
outside the tolerance. It computes HMAC-SHA256(secret, "<t>.<body>") in hex and
accepts if any `v1` matches, compared in constant time. On success it returns
the parsed event; on failure it raises `WebhookVerificationError`. The engine's
test vector is: secret `whsec_test`, t `1700000000`, body `{"hello":"world"}`,
giving `f592bbf3951cfc94e560eecfb5d9dd4da6b0fff2e626235f8ab4b54860925d0b`.

The client also has a `webhooks` resource, with these operations:
- endpoints: `create`, `list`, `get`, `update`, `delete`, `rotateSecret` and `test`;
- events: `list` (an auto-paging iterator, filterable by `type` and `account_id`), `get`, `deliveries` and `redeliver`.

### Merchants and cards

The runtime client has these resources:
- `merchants`: `create` (idempotent on `external_merchant_id`; returns the existing merchant on a repeat), `list`, `get` and `update`. Merchant accounts are opened with `accounts.open` and a `merchant_id`.
- `cardPrograms` (under a bank): `list`, `get`, `create` and `update`.
- `cards`: `issue` (an idempotent money call, because it may charge a fee; the result carries `card_number` and `cvv` once), `list`, `get`, `update`, `block`, `unblock`, `reportLost` (`lost` or `stolen`) and `close`.
- `cardAuthorizations`: `authorize`, `list`, `get`, `capture`, `release` and `refund`. Authorize, capture, release and refund are idempotent money calls and carry an idempotency key. `authorize` captures an approved payment in the same call by default, and its result then also carries `transaction`, `merchant_balance` and `total_debit_minor`; `capture: false` only reserves the money for a later `capture` or `release`.

A declined authorisation is a successful response with `authorization.status == "declined"` and a `decline_code`, not an exception. SDKs export the decline codes as constants.

### Savings and fixed deposits

The runtime client has these resources:
- `depositProducts` (under a bank): `list`, `get`, `create`, `update` and `quote`.
- `accounts.open`, which takes `product_id` (and optionally `interest_account_id`) to open a savings account.
- `accounts.interest`: the rate, accrued interest, next payout day and recent payouts.
- `fixedDeposits`:
  - `open` and `break`, which are idempotent money calls;
  - `list`, `get`, `update` (the maturity instruction) and `breakQuote`.

### Pagination
Account history is cursor-paginated. Each SDK offers a one-page call (`accounts.transactions`) and an iterator that follows `next_cursor` (`accounts.iterateTransactions`).

### Observability
Errors carry `requestId`. An optional response hook receives the status, `X-Request-Id` and whether the engine replayed an idempotent response (`Idempotent-Replayed`). The SDKs never log credentials.

## Contract tests

Every SDK has a contract test that runs the PRD §16.2 journey against a real engine: sign up, create a game and keys, create a bank, fund, name enquiry, transfer, idempotent replay, hold and capture, history, reversal, and a transfer whose first response is lost (it must post exactly once). It runs when `GBE_BASE_URL` is set and is skipped otherwise.

Each SDK repository has `ci/docker-compose.yml`, which starts PostgreSQL and the engine's published Docker image (`ghcr.io/showbaba/game-banking-engine`, override with `GBE_ENGINE_IMAGE`) on port 8080. Its CI runs the contract test against that, so an engine release that breaks an SDK fails the SDK's build.
