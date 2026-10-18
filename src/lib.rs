//! Server-side Rust SDK for the Game Banking Engine.
//!
//! **Keep game API keys on your servers.** A game API key moves money, so it
//! must never ship in a game client or a web page. The SDK never
//! authenticates players: your server verifies its own player, then calls
//! the engine for them.
//!
//! There are two clients:
//!
//! * [`GameBank`], with a game API key (`gbe_test_sk_…` sandbox,
//!   `gbe_live_sk_…` production), for everything your game server does at
//!   runtime.
//! * [`GameBankManagement`], with a developer credential (`gbe_dev_…`) or a
//!   sign-in session token, for setup and CI: games, API keys, developer
//!   credentials and audit logs.
//!
//! ```no_run
//! use gamebanking::{ErrorKind, FundingParams, GameBank, OpenAccountParams, TransferParams, UpsertPlayerParams};
//!
//! # async fn run(bank_id: String) -> gamebanking::Result<()> {
//! let gb = GameBank::builder()
//!     .api_key(std::env::var("GBE_API_KEY").unwrap())
//!     .base_url("https://api.gamebank.dev")
//!     .build()?;
//!
//! let player = gb
//!     .players()
//!     .upsert(&UpsertPlayerParams { external_player_id: "user-42".into(), display_name: Some("Ada Obi".into()), ..Default::default() })
//!     .await?;
//! let account = gb.accounts().open(&OpenAccountParams { bank_id, player_id: Some(player.id), ..Default::default() }).await?;
//!
//! // Your stable ID as the idempotency key: retrying never pays twice.
//! gb.funding()
//!     .idempotency_key("payroll-2040-06-01-user-42")
//!     .create(&FundingParams {
//!         account_id: account.id.clone(),
//!         amount_minor: 500_000,
//!         reason: "salary".into(),
//!         reference: "payroll-2040-06-01-user-42".into(),
//!         ..Default::default()
//!     })
//!     .await?;
//!
//! let sent = gb
//!     .transfers()
//!     .create(&TransferParams {
//!         source_account_id: account.id.clone(),
//!         destination_account_number: Some("5915945174".into()),
//!         amount_minor: 1_000,
//!         ..Default::default()
//!     })
//!     .await;
//! match sent {
//!     Err(e) if e.kind() == ErrorKind::InsufficientFunds => println!("available {:?}", e.details.get("available_balance")),
//!     other => {
//!         other?;
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! Money-moving calls send an `Idempotency-Key`: your own (set with
//! `.idempotency_key(…)` on the resource) or one generated once per call and
//! reused on the SDK's own retries.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]
// Error carries the engine's details and request ID inline; its size is
// irrelevant next to a network round trip.
#![allow(clippy::result_large_err)]

mod client;
mod error;
mod management;
mod transport;

pub mod cards;
pub mod money;
pub mod savings;
pub mod types;
pub mod webhooks;

pub use client::{
    Accounts, Banks, ClockResource, Currencies, Fees, Funding, GameBank, Holds, Pager, Players,
    Transactions, Transfers,
};
pub use error::{Error, ErrorKind, Result};
pub use management::{ApiKeys, AuditLogParams, DeveloperCredentials, GameBankManagement, Games};
pub use money::{format_minor, to_minor};
pub use transport::{
    retry_delay, BoxFuture, ClientBuilder, Environment, FromInner, HttpRequest, HttpResponse,
    HttpTransport, RequestOptions, ReqwestTransport, ResponseInfo, TransportError, VERSION,
};
pub use types::*;
pub use webhooks::{verify_webhook, WebhookVerificationError, WebhookVerifier};

#[cfg(test)]
mod tests;
