//! Merchants, card programs, debit cards and card payments.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::client::{reason_body, resource};
use crate::error::Result;
use crate::transport::{api_path, check_amount, Op, Query};
use crate::types::{
    nullable, Balance, CurrencyBrief, FeeCharge, List, Metadata, Timestamp, Transaction,
};

/// A business in the game: an NPC shop or a player-run store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Merchant {
    pub id: String,
    pub external_merchant_id: String,
    pub name: String,
    pub category: String,
    pub owner_player_id: Option<String>,
    /// `active` or `suspended` (card payments are declined).
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    pub metadata: Metadata,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateMerchantParams {
    /// Your own ID; creating it again returns the existing merchant.
    pub external_merchant_id: String,
    pub name: String,
    /// Defaults to `general`.
    pub category: Option<String>,
    pub owner_player_id: Option<String>,
    pub metadata: Option<Metadata>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateMerchantParams {
    pub name: Option<String>,
    pub category: Option<String>,
    pub status: Option<String>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Default)]
pub struct ListMerchantsParams {
    pub status: Option<String>,
    pub owner_player_id: Option<String>,
    pub external_merchant_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CardProgram {
    pub id: String,
    pub bank_id: String,
    pub name: String,
    pub bin: String,
    pub validity_months: i32,
    /// How long an authorisation holds funds, in game seconds.
    pub auth_expiry_seconds: i64,
    pub per_txn_limit_minor: Option<i64>,
    pub daily_limit_minor: Option<i64>,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Creates or updates a program; `None` fields are unchanged (or defaulted).
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CardProgramParams {
    pub name: Option<String>,
    /// Create only.
    pub bin: Option<String>,
    pub validity_months: Option<i32>,
    pub auth_expiry_seconds: Option<i64>,
    pub per_txn_limit_minor: Option<i64>,
    pub daily_limit_minor: Option<i64>,
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clear: Vec<String>,
}

/// Card statuses.
pub mod card_status {
    pub const ACTIVE: &str = "active";
    pub const BLOCKED: &str = "blocked";
    pub const LOST: &str = "lost";
    pub const STOLEN: &str = "stolen";
    pub const EXPIRED: &str = "expired";
    pub const CLOSED: &str = "closed";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    pub id: String,
    pub bank_id: String,
    pub account_id: String,
    pub player_id: String,
    pub program_id: String,
    pub masked_number: String,
    pub last4: String,
    pub expiry_month: i32,
    /// Valid through the end of `expiry_month` of game time.
    pub expiry_year: i32,
    pub name_on_card: String,
    pub status: String,
    pub status_reason: Option<String>,
    /// The card's own limits; `None` uses the program's.
    pub per_txn_limit_minor: Option<i64>,
    pub daily_limit_minor: Option<i64>,
    pub effective_per_txn_limit_minor: Option<i64>,
    pub effective_daily_limit_minor: Option<i64>,
    #[serde(deserialize_with = "nullable")]
    pub metadata: Metadata,
    pub status_changed_at: Timestamp,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct IssueCardParams {
    /// A player's current account.
    pub account_id: String,
    pub program_id: Option<String>,
    pub name_on_card: Option<String>,
    pub per_txn_limit_minor: Option<i64>,
    pub daily_limit_minor: Option<i64>,
    pub max_fee_minor: Option<i64>,
    pub metadata: Option<Metadata>,
}

/// Carries `card_number` and `cvv`, shown only here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IssueCardResult {
    pub card: Card,
    pub card_number: String,
    pub cvv: String,
    #[serde(deserialize_with = "nullable")]
    pub fees: Vec<FeeCharge>,
    pub fees_total_minor: i64,
    pub balance: Balance,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateCardParams {
    pub name_on_card: Option<String>,
    pub per_txn_limit_minor: Option<i64>,
    pub daily_limit_minor: Option<i64>,
    /// Back to the program's limit, e.g. `vec!["daily_limit_minor".into()]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clear: Vec<String>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Default)]
pub struct ListCardsParams {
    pub account_id: Option<String>,
    pub player_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
}

/// Decline codes on a declined authorisation.
pub mod decline_code {
    pub const INVALID_EXPIRY: &str = "invalid_expiry";
    pub const INVALID_CVV: &str = "invalid_cvv";
    pub const CARD_BLOCKED: &str = "card_blocked";
    pub const CARD_LOST: &str = "card_lost";
    pub const CARD_STOLEN: &str = "card_stolen";
    pub const CARD_EXPIRED: &str = "card_expired";
    pub const CARD_CLOSED: &str = "card_closed";
    pub const ACCOUNT_FROZEN: &str = "account_frozen";
    pub const ACCOUNT_CLOSED: &str = "account_closed";
    pub const MERCHANT_INACTIVE: &str = "merchant_inactive";
    pub const EXCEEDS_TRANSACTION_LIMIT: &str = "exceeds_transaction_limit";
    pub const EXCEEDS_DAILY_LIMIT: &str = "exceeds_daily_limit";
    pub const INSUFFICIENT_FUNDS: &str = "insufficient_funds";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CardRefund {
    pub id: String,
    pub transaction_id: String,
    pub amount_minor: i64,
    pub reason: Option<String>,
    pub game_time: Timestamp,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthorizationMerchant {
    pub id: String,
    pub name: String,
    pub category: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CardAuthorization {
    pub id: String,
    pub card_id: String,
    pub card_last4: String,
    pub bank_id: String,
    pub account_id: String,
    pub merchant: AuthorizationMerchant,
    pub merchant_account_id: String,
    /// `approved`, `declined`, `captured`, `released`, `expired` or `refunded`.
    pub status: String,
    pub decline_code: Option<String>,
    pub decline_message: Option<String>,
    pub amount_minor: i64,
    pub captured_minor: i64,
    pub refunded_minor: i64,
    pub currency: CurrencyBrief,
    pub hold_id: Option<String>,
    pub reserved_fee_minor: i64,
    pub expires_at_game: Option<Timestamp>,
    pub capture_transaction_id: Option<String>,
    pub reference: Option<String>,
    pub description: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub refunds: Vec<CardRefund>,
    pub game_time: Timestamp,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub closed_at: Option<Timestamp>,
}

impl CardAuthorization {
    /// The [`decline_code`] when the payment was declined.
    pub fn declined(&self) -> Option<&str> {
        if self.status == "declined" {
            self.decline_code.as_deref()
        } else {
            None
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct AuthorizeCardParams {
    /// The card: `card_id`, or `card_number` with `expiry_month` and `expiry_year` (and optionally `cvv`).
    pub card_id: Option<String>,
    pub card_number: Option<String>,
    pub expiry_month: Option<i32>,
    pub expiry_year: Option<i32>,
    pub cvv: Option<String>,
    /// A merchant account in the card's bank.
    pub merchant_account_id: Option<String>,
    pub merchant_account_number: Option<String>,
    pub amount_minor: i64,
    pub reference: Option<String>,
    pub description: Option<String>,
    pub max_fee_minor: Option<i64>,
    /// `None` or `Some(true)` (the default) captures an approved payment in full in the
    /// same call; `Some(false)` only reserves the money for a later capture or release.
    pub capture: Option<bool>,
}

/// A decline is a result, not an error: see [`CardAuthorization::declined`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthorizeCardResult {
    pub authorization: CardAuthorization,
    #[serde(deserialize_with = "nullable")]
    pub fees: Vec<FeeCharge>,
    pub fees_total_minor: i64,
    /// The cardholder's account after the call.
    pub balance: Balance,
    /// The `card_payment`, when the payment was captured (approved, without `capture: Some(false)`).
    pub transaction: Option<Transaction>,
    pub merchant_balance: Option<Balance>,
    pub total_debit_minor: Option<i64>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CaptureCardParams {
    /// `None` captures everything; the rest is released.
    pub amount_minor: Option<i64>,
    pub max_fee_minor: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureCardResult {
    pub authorization: CardAuthorization,
    pub transaction: Transaction,
    pub card_balance: Balance,
    pub merchant_balance: Balance,
    #[serde(deserialize_with = "nullable")]
    pub fees: Vec<FeeCharge>,
    pub fees_total_minor: i64,
    pub total_debit_minor: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReleaseCardResult {
    pub authorization: CardAuthorization,
    pub balance: Balance,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct RefundCardParams {
    /// `None` refunds the rest.
    pub amount_minor: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RefundCardResult {
    pub refund: CardRefund,
    pub authorization: CardAuthorization,
    pub transaction: Transaction,
    pub card_balance: Balance,
    pub merchant_balance: Balance,
}

#[derive(Debug, Clone, Default)]
pub struct ListCardAuthorizationsParams {
    pub card_id: Option<String>,
    pub merchant_id: Option<String>,
    pub account_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
}

resource!(
    /// Merchants. Open a merchant's account with `accounts().open` and a `merchant_id`.
    Merchants
);

impl Merchants {
    /// Registers a merchant; repeat calls with the same `external_merchant_id`
    /// return it. Needs `merchants:write`.
    pub async fn create(&self, p: &CreateMerchantParams) -> Result<Merchant> {
        self.t
            .call(
                Op::post("/v1/merchants".into()).json(p).retry_safe(),
                &self.opts,
            )
            .await
    }

    pub async fn list(&self, p: &ListMerchantsParams) -> Result<Vec<Merchant>> {
        let q = Query::new()
            .opt("status", p.status.as_ref())
            .opt("owner_player_id", p.owner_player_id.as_ref())
            .opt("external_merchant_id", p.external_merchant_id.as_ref())
            .opt("limit", p.limit);
        let out: List<Merchant> = self
            .t
            .call(Op::get("/v1/merchants".into()).query(q), &self.opts)
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, merchant_id: &str) -> Result<Merchant> {
        self.t
            .call(
                Op::get(api_path!("/v1/merchants/{}", merchant_id)),
                &self.opts,
            )
            .await
    }

    /// Renames, recategorises, suspends or reactivates a merchant.
    pub async fn update(&self, merchant_id: &str, p: &UpdateMerchantParams) -> Result<Merchant> {
        self.t
            .call(
                Op::patch(api_path!("/v1/merchants/{}", merchant_id)).json(p),
                &self.opts,
            )
            .await
    }
}

resource!(
    /// A bank's card programs.
    CardPrograms
);

impl CardPrograms {
    pub async fn list(&self, bank_id: &str) -> Result<Vec<CardProgram>> {
        let out: List<CardProgram> = self
            .t
            .call(
                Op::get(api_path!("/v1/banks/{}/card-programs", bank_id)),
                &self.opts,
            )
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, bank_id: &str, program_id: &str) -> Result<CardProgram> {
        self.t
            .call(
                Op::get(api_path!(
                    "/v1/banks/{}/card-programs/{}",
                    bank_id,
                    program_id
                )),
                &self.opts,
            )
            .await
    }

    /// Adds a program. Turn on the bank's `cards` feature flag to issue cards. Needs `bank:write`.
    pub async fn create(&self, bank_id: &str, p: &CardProgramParams) -> Result<CardProgram> {
        self.t
            .call(
                Op::post(api_path!("/v1/banks/{}/card-programs", bank_id)).json(p),
                &self.opts,
            )
            .await
    }

    pub async fn update(
        &self,
        bank_id: &str,
        program_id: &str,
        p: &CardProgramParams,
    ) -> Result<CardProgram> {
        self.t
            .call(
                Op::patch(api_path!(
                    "/v1/banks/{}/card-programs/{}",
                    bank_id,
                    program_id
                ))
                .json(p),
                &self.opts,
            )
            .await
    }
}

resource!(
    /// Issues and controls debit cards.
    Cards
);

impl Cards {
    /// Creates a card on a player's current account, charging any
    /// `card_issuance` fee. `card_number` and `cvv` are in the result once.
    /// Needs `cards:write`.
    pub async fn issue(&self, p: &IssueCardParams) -> Result<IssueCardResult> {
        self.t
            .call(
                Op::post("/v1/cards".into()).json(p).idempotent(),
                &self.opts,
            )
            .await
    }

    pub async fn list(&self, p: &ListCardsParams) -> Result<Vec<Card>> {
        let q = Query::new()
            .opt("account_id", p.account_id.as_ref())
            .opt("player_id", p.player_id.as_ref())
            .opt("status", p.status.as_ref())
            .opt("limit", p.limit);
        let out: List<Card> = self
            .t
            .call(Op::get("/v1/cards".into()).query(q), &self.opts)
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, card_id: &str) -> Result<Card> {
        self.t
            .call(Op::get(api_path!("/v1/cards/{}", card_id)), &self.opts)
            .await
    }

    /// Changes the name on the card or its own limits.
    pub async fn update(&self, card_id: &str, p: &UpdateCardParams) -> Result<Card> {
        self.t
            .call(
                Op::patch(api_path!("/v1/cards/{}", card_id)).json(p),
                &self.opts,
            )
            .await
    }

    async fn status(&self, card_id: &str, action: &str, reason: Option<&str>) -> Result<Card> {
        let op = Op::post(format!("{}/{action}", api_path!("/v1/cards/{}", card_id)))
            .json(&reason_body(reason));
        self.t.call(op, &self.opts).await
    }

    /// Pauses the card: new payments are declined (`card_blocked`).
    pub async fn block(&self, card_id: &str, reason: Option<&str>) -> Result<Card> {
        self.status(card_id, "block", reason).await
    }

    pub async fn unblock(&self, card_id: &str) -> Result<Card> {
        self.status(card_id, "unblock", None).await
    }

    /// Reports the card `lost` or `stolen` (the `reason`): final, and no
    /// replacement is issued. Payments approved before the report still capture.
    pub async fn report_lost(&self, card_id: &str, reason: &str) -> Result<Card> {
        self.status(card_id, "report-lost", Some(reason)).await
    }

    pub async fn close(&self, card_id: &str, reason: Option<&str>) -> Result<Card> {
        self.status(card_id, "close", reason).await
    }
}

resource!(
    /// Card payments.
    CardAuthorizations
);

impl CardAuthorizations {
    /// Pays a merchant by card. An approved payment is captured at once, or
    /// with `capture: Some(false)` only reserved for a later `capture` or
    /// `release`. A decline is not an error: check
    /// `result.authorization.declined()`. Needs `cards:charge`.
    pub async fn authorize(&self, p: &AuthorizeCardParams) -> Result<AuthorizeCardResult> {
        check_amount(p.amount_minor, true)?;
        self.t
            .call(
                Op::post("/v1/card-authorizations".into())
                    .json(p)
                    .idempotent(),
                &self.opts,
            )
            .await
    }

    pub async fn list(&self, p: &ListCardAuthorizationsParams) -> Result<Vec<CardAuthorization>> {
        let q = Query::new()
            .opt("card_id", p.card_id.as_ref())
            .opt("merchant_id", p.merchant_id.as_ref())
            .opt("account_id", p.account_id.as_ref())
            .opt("status", p.status.as_ref())
            .opt("limit", p.limit);
        let out: List<CardAuthorization> = self
            .t
            .call(
                Op::get("/v1/card-authorizations".into()).query(q),
                &self.opts,
            )
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, authorization_id: &str) -> Result<CardAuthorization> {
        self.t
            .call(
                Op::get(api_path!("/v1/card-authorizations/{}", authorization_id)),
                &self.opts,
            )
            .await
    }

    /// Settles all or part of an approved payment into the merchant account; the rest is released.
    pub async fn capture(
        &self,
        authorization_id: &str,
        p: &CaptureCardParams,
    ) -> Result<CaptureCardResult> {
        check_amount(p.amount_minor.unwrap_or(0), false)?;
        let op = Op::post(api_path!(
            "/v1/card-authorizations/{}/capture",
            authorization_id
        ))
        .json(p)
        .idempotent();
        self.t.call(op, &self.opts).await
    }

    /// Cancels an approved payment and frees its funds.
    pub async fn release(
        &self,
        authorization_id: &str,
        reason: Option<&str>,
    ) -> Result<ReleaseCardResult> {
        let op = Op::post(api_path!(
            "/v1/card-authorizations/{}/release",
            authorization_id
        ))
        .json(&reason_body(reason))
        .idempotent();
        self.t.call(op, &self.opts).await
    }

    /// Refunds all or part of a captured payment from the merchant account. Fees are not refunded.
    pub async fn refund(
        &self,
        authorization_id: &str,
        p: &RefundCardParams,
    ) -> Result<RefundCardResult> {
        check_amount(p.amount_minor.unwrap_or(0), false)?;
        let op = Op::post(api_path!(
            "/v1/card-authorizations/{}/refunds",
            authorization_id
        ))
        .json(p)
        .idempotent();
        self.t.call(op, &self.opts).await
    }
}
