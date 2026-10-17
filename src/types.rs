//! Request and response types. Amounts are `i64` minor units; a currency's
//! `exponent` is its decimal places. Times are UTC; game days are `YYYY-MM-DD`.
//!
//! Request parameters derive `Default`: set what you need and finish with
//! `..Default::default()`. Unset (`None`) fields are left out, so the engine's
//! defaults apply.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;

/// A UTC timestamp.
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// Free-form JSON metadata.
pub type Metadata = Map<String, Value>;

/// Treats `null` like a missing field.
pub(crate) fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// One page of a cursor-paginated list. Pass `next_cursor` back for the next page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de>"))]
pub struct Page<T> {
    #[serde(deserialize_with = "nullable")]
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Page {
            data: Vec::new(),
            next_cursor: None,
            has_more: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, bound(deserialize = "T: Deserialize<'de>"))]
pub(crate) struct List<T> {
    #[serde(deserialize_with = "nullable")]
    pub data: Vec<T>,
}

impl<T> Default for List<T> {
    fn default() -> Self {
        List { data: Vec::new() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CurrencyBrief {
    pub code: String,
    pub exponent: u32,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Currency {
    pub id: String,
    pub code: String,
    pub name: String,
    pub exponent: u32,
    pub symbol: Option<String>,
    pub standard: bool,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub id: String,
    pub bank_id: String,
    pub bank_code: String,
    /// `player`, `merchant` or `system`.
    pub owner_type: String,
    pub player_id: Option<String>,
    /// Set on merchant accounts.
    pub merchant_id: Option<String>,
    /// The deposit product of a savings or fixed-deposit account.
    pub product_id: Option<String>,
    /// Receives the account's interest; `None` pays the account itself.
    pub interest_account_id: Option<String>,
    pub system_role: Option<String>,
    /// `current`, `savings`, `fixed_deposit`, …
    pub account_type: String,
    pub account_number: String,
    pub account_name: String,
    pub currency: CurrencyBrief,
    pub status: String,
    pub allow_negative_balance: bool,
    pub posted_balance: i64,
    pub available_balance: i64,
    pub held_balance: i64,
    pub version: i64,
    #[serde(deserialize_with = "nullable")]
    pub metadata: Metadata,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Bank {
    pub id: String,
    pub environment: String,
    pub name: String,
    pub slug: String,
    pub bank_code: String,
    pub base_currency: Currency,
    #[serde(deserialize_with = "nullable")]
    pub branding: Metadata,
    #[serde(deserialize_with = "nullable")]
    pub feature_flags: BTreeMap<String, bool>,
    pub config_version: i32,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Set when a bank is created or fetched by ID.
    #[serde(deserialize_with = "nullable")]
    pub system_accounts: Vec<Account>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Balance {
    pub account_id: String,
    pub account_number: String,
    pub currency: CurrencyBrief,
    pub posted_balance: i64,
    pub available_balance: i64,
    pub held_balance: i64,
    pub posted_display: String,
    pub available_display: String,
    pub version: i64,
    pub as_of: Timestamp,
    pub as_of_game_time: Option<Timestamp>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Player {
    pub id: String,
    pub external_player_id: String,
    pub display_name: Option<String>,
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    pub metadata: Metadata,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Ledger line kinds.
pub mod entry_kind {
    pub const PRINCIPAL: &str = "principal";
    pub const FEE: &str = "fee";
    pub const LEVY: &str = "levy";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub account_id: String,
    pub account_number: String,
    pub account_name: String,
    /// `debit` or `credit`.
    pub side: String,
    pub amount_minor: i64,
    pub balance_after: i64,
    /// `principal`, `fee` or `levy`. Fee and levy lines carry the rule and its name.
    pub kind: String,
    pub fee_rule_id: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Initiator {
    pub r#type: String,
    pub id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Transaction {
    pub id: String,
    pub session_id: String,
    /// `funding`, `transfer`, `reversal`, `card_payment`, `interest`, …
    pub r#type: String,
    pub status: String,
    pub bank_id: String,
    pub currency: CurrencyBrief,
    pub amount_minor: i64,
    pub amount_display: String,
    pub reference: Option<String>,
    pub narration: Option<String>,
    pub initiated_by: Initiator,
    pub related_transaction_id: Option<String>,
    #[serde(rename = "reversed_by_transaction_id")]
    pub reversed_by: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub metadata: Metadata,
    pub created_at: Timestamp,
    pub posted_at: Option<Timestamp>,
    pub game_time: Timestamp,
    #[serde(deserialize_with = "nullable")]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Counterparty {
    pub account_id: String,
    pub account_number: String,
    pub account_name: String,
    pub bank_code: String,
}

/// One line of an account's statement. `direction` is `debit` or `credit`
/// from the account holder's point of view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryItem {
    pub entry_id: String,
    pub transaction_id: String,
    pub session_id: String,
    pub r#type: String,
    pub status: String,
    pub direction: String,
    pub amount_minor: i64,
    pub amount_display: String,
    pub balance_after: i64,
    /// `principal`, `fee` or `levy`. A fee or levy line's narration is the fee's name.
    pub kind: String,
    pub narration: Option<String>,
    pub reference: Option<String>,
    pub counterparty: Option<Counterparty>,
    pub real_time: Timestamp,
    pub game_time: Timestamp,
    pub related_transaction_id: Option<String>,
}

/// One page of account history.
pub type HistoryPage = Page<HistoryItem>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Hold {
    pub id: String,
    pub bank_id: String,
    pub account_id: String,
    pub destination_account_id: Option<String>,
    pub amount_minor: i64,
    /// Held on top of the amount for the capture's expected fees.
    pub reserved_fee_minor: i64,
    pub captured_amount_minor: Option<i64>,
    pub status: String,
    pub reason: String,
    pub reference: Option<String>,
    pub expires_at_game: Timestamp,
    pub capture_transaction_id: Option<String>,
    pub created_at: Timestamp,
    pub created_game_time: Timestamp,
    pub closed_at: Option<Timestamp>,
    pub closed_game_time: Option<Timestamp>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NameEnquiry {
    pub bank_code: String,
    pub bank_name: String,
    pub account_number: String,
    pub account_name: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    /// `real_time`, `scaled` or `manual`.
    pub mode: String,
    pub speed: i32,
    pub time_zone: String,
    pub game_time: Timestamp,
    pub game_day: String,
    pub real_time: Timestamp,
}

/// Caps one limit scope; `None` means no limit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitPolicy {
    pub per_transaction_max: Option<i64>,
    pub daily_max: Option<i64>,
}

/// Limit scopes.
pub mod limit_scope {
    pub const PLAYER_FUNDING: &str = "player_funding";
    pub const GAME_FUNDING: &str = "game_funding";
    pub const ACCOUNT_TRANSFER: &str = "account_transfer";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub bank_id: String,
    pub currency: String,
    #[serde(deserialize_with = "nullable")]
    pub limits: BTreeMap<String, Option<LimitPolicy>>,
    pub game_day: String,
    pub game_funding_used_today: i64,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReconciliationReport {
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    pub accounts_checked: i64,
    pub transactions_checked: i64,
    #[serde(deserialize_with = "nullable")]
    pub differences: Vec<Metadata>,
    pub ok: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FundingResult {
    pub transaction: Transaction,
    pub balance: Balance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TransferResult {
    pub transaction: Transaction,
    pub source_balance: Balance,
    pub destination_balance: Option<Balance>,
    /// Fees and levies charged to the sender on top of the amount.
    #[serde(deserialize_with = "nullable")]
    pub fees: Vec<FeeCharge>,
    pub fees_total_minor: i64,
    /// The amount plus fees.
    pub total_debit_minor: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HoldResult {
    pub hold: Hold,
    pub balance: Balance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureResult {
    pub hold: Hold,
    pub transaction: Transaction,
    pub source_balance: Balance,
    pub destination_balance: Option<Balance>,
    #[serde(deserialize_with = "nullable")]
    pub fees: Vec<FeeCharge>,
    pub fees_total_minor: i64,
    pub total_debit_minor: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockJobs {
    pub game_day: String,
    pub holds_expired: i64,
    #[serde(deserialize_with = "nullable")]
    pub jobs_run: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvanceClockResult {
    pub clock: Clock,
    pub previous_game_time: Timestamp,
    pub jobs: ClockJobs,
}

// ---- request parameters ----

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpsertPlayerParams {
    /// Your own player ID.
    pub external_player_id: String,
    pub display_name: Option<String>,
    pub metadata: Option<Metadata>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct OpenAccountParams {
    pub bank_id: String,
    /// Opens a player account.
    pub player_id: Option<String>,
    /// Opens a merchant account, which receives card payments.
    pub merchant_id: Option<String>,
    /// Makes a savings account that earns interest under this product.
    pub product_id: Option<String>,
    /// Pays the savings account's interest into another of the player's accounts.
    pub interest_account_id: Option<String>,
    pub account_type: Option<String>,
    pub account_name: Option<String>,
    pub currency: Option<String>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone, Default)]
pub struct ListAccountsParams {
    pub bank_id: Option<String>,
    pub player_id: Option<String>,
    pub merchant_id: Option<String>,
    pub include_system: bool,
    pub limit: Option<u32>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateAccountStatusParams {
    /// `active`, `frozen` or `closed`.
    pub status: String,
    pub reason: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct FundingParams {
    pub account_id: String,
    pub amount_minor: i64,
    pub reason: String,
    /// Your own unique ID for this issuance.
    pub reference: String,
    pub currency: Option<String>,
    pub source_account_id: Option<String>,
    pub narration: Option<String>,
    pub metadata: Option<Metadata>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct TransferParams {
    pub source_account_id: String,
    pub amount_minor: i64,
    pub destination_account_id: Option<String>,
    pub destination_account_number: Option<String>,
    pub destination_bank_code: Option<String>,
    pub bank_id: Option<String>,
    pub currency: Option<String>,
    pub reference: Option<String>,
    pub narration: Option<String>,
    pub metadata: Option<Metadata>,
    /// Refuses the transfer with [`ErrorKind::FeeChanged`](crate::ErrorKind::FeeChanged)
    /// if its fees now total more, e.g. the `fees_total_minor` of a quote the player accepted.
    pub max_fee_minor: Option<i64>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlaceHoldParams {
    pub account_id: String,
    pub amount_minor: i64,
    pub reason: String,
    pub reference: Option<String>,
    pub destination_account_id: Option<String>,
    pub destination_account_number: Option<String>,
    pub expires_at_game: Option<Timestamp>,
    pub expires_in_game_seconds: Option<i64>,
    /// Caps the capture fee reserved now (`FeeChanged` if higher).
    pub max_fee_minor: Option<i64>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CaptureHoldParams {
    /// The amount to capture; `None` captures the whole hold.
    pub amount_minor: Option<i64>,
    pub destination_account_id: Option<String>,
    pub destination_account_number: Option<String>,
    pub narration: Option<String>,
    pub reference: Option<String>,
    pub metadata: Option<Metadata>,
    /// Refuses the capture with `FeeChanged` if its fees now total more.
    pub max_fee_minor: Option<i64>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReverseParams {
    pub reason: String,
    pub narration: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateBankParams {
    pub name: String,
    pub base_currency: String,
    pub slug: Option<String>,
    pub bank_code: Option<String>,
    pub branding: Option<Metadata>,
    pub feature_flags: Option<BTreeMap<String, bool>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateBankParams {
    pub name: Option<String>,
    pub branding: Option<Metadata>,
    /// e.g. `{"cards": true}` or `{"savings": true}`.
    pub feature_flags: Option<BTreeMap<String, bool>>,
    pub status: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateCurrencyParams {
    pub code: String,
    pub name: String,
    pub exponent: u32,
    pub symbol: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct ConfigureClockParams {
    pub mode: Option<String>,
    pub speed: Option<i32>,
    pub time_zone: Option<String>,
    pub game_time: Option<Timestamp>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct AdvanceClockParams {
    pub seconds: Option<i64>,
    pub minutes: Option<i64>,
    pub hours: Option<i64>,
    pub days: Option<i64>,
    pub to: Option<Timestamp>,
}

// ---- fees and charges ----

/// Fee events.
pub mod fee_event {
    pub const TRANSFER: &str = "transfer";
    pub const HOLD_CAPTURE: &str = "hold_capture";
    pub const ACCOUNT_MAINTENANCE: &str = "account_maintenance";
    pub const CARD_ISSUANCE: &str = "card_issuance";
    pub const CARD_PAYMENT: &str = "card_payment";
    pub const SAVINGS_EXCESS_WITHDRAWAL: &str = "savings_excess_withdrawal";
}

/// Fee kinds.
pub mod fee_kind {
    /// Credits the bank's `fee_income` account.
    pub const FEE: &str = "fee";
    /// Credits `levies_payable`: money held for someone else.
    pub const LEVY: &str = "levy";
}

/// Fee models.
pub mod fee_model {
    pub const FIXED: &str = "fixed";
    pub const PERCENTAGE: &str = "percentage";
    pub const COMBINED: &str = "combined";
}

/// Where a fee rule stands at the current game time.
pub mod fee_state {
    pub const IN_EFFECT: &str = "in_effect";
    pub const SCHEDULED: &str = "scheduled";
    pub const ENDED: &str = "ended";
    pub const INACTIVE: &str = "inactive";
}

/// A fee or levy a bank charges. Times are game time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FeeRule {
    pub id: String,
    pub bank_id: String,
    pub code: String,
    pub name: String,
    pub event: String,
    pub kind: String,
    pub model: String,
    pub fixed_minor: i64,
    /// 100 = 1%.
    pub percentage_bps: i64,
    pub min_fee_minor: Option<i64>,
    pub max_fee_minor: Option<i64>,
    pub min_amount_minor: Option<i64>,
    pub max_amount_minor: Option<i64>,
    pub payer: String,
    pub credit_account: String,
    pub status: String,
    pub effective_from: Timestamp,
    pub effective_to: Option<Timestamp>,
    pub version: i32,
    /// Only `in_effect` rules charge.
    pub state: String,
    #[serde(deserialize_with = "nullable")]
    pub warnings: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateFeeRuleParams {
    pub code: String,
    pub name: String,
    pub event: String,
    pub model: String,
    pub kind: Option<String>,
    pub fixed_minor: Option<i64>,
    pub percentage_bps: Option<i64>,
    pub min_fee_minor: Option<i64>,
    pub max_fee_minor: Option<i64>,
    pub min_amount_minor: Option<i64>,
    pub max_amount_minor: Option<i64>,
    pub status: Option<String>,
    pub effective_from: Option<Timestamp>,
    pub effective_to: Option<Timestamp>,
    /// Starts the fee at the current game time.
    pub start_now: Option<bool>,
}

/// Changes a fee; `None` fields keep their value. `clear` resets fields to
/// "no limit", e.g. `vec!["max_fee_minor".into()]`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateFeeRuleParams {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub model: Option<String>,
    pub fixed_minor: Option<i64>,
    pub percentage_bps: Option<i64>,
    pub min_fee_minor: Option<i64>,
    pub max_fee_minor: Option<i64>,
    pub min_amount_minor: Option<i64>,
    pub max_amount_minor: Option<i64>,
    pub status: Option<String>,
    pub effective_from: Option<Timestamp>,
    pub effective_to: Option<Timestamp>,
    pub start_now: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clear: Vec<String>,
}

/// One fee or levy charged on a payment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FeeCharge {
    pub fee_rule_id: String,
    pub fee_rule_version: i32,
    pub code: String,
    pub name: String,
    pub kind: String,
    pub amount_minor: i64,
    pub credit_account: String,
}

/// Asks what a payment would cost now.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct QuoteFeesParams {
    pub bank_id: String,
    pub amount_minor: i64,
    /// A [`fee_event`]; the engine defaults to `transfer`.
    pub event: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FeeQuote {
    pub bank_id: String,
    pub event: String,
    pub amount_minor: i64,
    #[serde(deserialize_with = "nullable")]
    pub fees: Vec<FeeCharge>,
    pub fees_total_minor: i64,
    pub total_debit_minor: i64,
    pub game_time: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BookAccount {
    pub system_role: String,
    pub account_name: String,
    pub account_number: String,
    pub balance_minor: i64,
    pub balance_display: String,
}

/// The bank's own ledger, from which its profit and loss derives.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BankBooks {
    pub bank_id: String,
    pub currency: String,
    pub fee_income_minor: i64,
    pub levies_payable_minor: i64,
    /// Interest paid to savers (negative).
    pub interest_expense_minor: i64,
    /// Interest earned by savers, not yet paid.
    pub accrued_interest_minor: i64,
    /// Money in active fixed deposits.
    pub fixed_deposits_minor: i64,
    #[serde(deserialize_with = "nullable")]
    pub accounts: Vec<BookAccount>,
    pub as_of: Timestamp,
}

// ---- management ----

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Developer {
    pub id: String,
    pub name: String,
    pub contact_email: String,
    pub status: String,
    pub has_password: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    pub id: String,
    pub developer_id: String,
    pub name: String,
    pub slug: String,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiKey {
    pub id: String,
    pub game_id: String,
    pub environment: String,
    pub label: String,
    pub prefix: String,
    /// Set only in the response that created the key.
    pub secret: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub scopes: Vec<String>,
    pub last_used_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DeveloperCredential {
    pub id: String,
    pub label: String,
    pub prefix: String,
    /// Set only in the response that created the credential.
    pub secret: Option<String>,
    pub last_used_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AuditLog {
    pub id: String,
    pub developer_id: Option<String>,
    pub game_id: Option<String>,
    pub environment: Option<String>,
    pub actor_type: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub correlation_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub metadata: Metadata,
    pub created_at: Timestamp,
}

/// Renames a game or changes its slug or status; `None` fields keep their value.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateGameParams {
    pub name: Option<String>,
    pub slug: Option<String>,
    /// `active` or `suspended`.
    pub status: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct IssueApiKeyParams {
    /// `sandbox` or `production`.
    pub environment: String,
    pub label: Option<String>,
    /// Defaults to `bank:read`, `account:read`, `account:write`, `transfer:create`.
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<Timestamp>,
}

/// API key scopes.
pub mod scope {
    pub const BANK_READ: &str = "bank:read";
    pub const BANK_WRITE: &str = "bank:write";
    pub const ACCOUNT_READ: &str = "account:read";
    pub const ACCOUNT_WRITE: &str = "account:write";
    pub const TRANSFER_CREATE: &str = "transfer:create";
    pub const FUNDING_CREATE: &str = "funding:create";
    pub const TRANSACTION_REVERSE: &str = "transaction:reverse";
    pub const LIMITS_WRITE: &str = "limits:write";
    pub const WEBHOOKS_READ: &str = "webhooks:read";
    pub const WEBHOOKS_WRITE: &str = "webhooks:write";
    pub const MERCHANTS_WRITE: &str = "merchants:write";
    pub const CARDS_READ: &str = "cards:read";
    pub const CARDS_WRITE: &str = "cards:write";
    pub const CARDS_CHARGE: &str = "cards:charge";
    pub const DEPOSITS_WRITE: &str = "deposits:write";

    /// Every scope.
    pub const ALL: [&str; 15] = [
        BANK_READ,
        BANK_WRITE,
        ACCOUNT_READ,
        ACCOUNT_WRITE,
        TRANSFER_CREATE,
        FUNDING_CREATE,
        TRANSACTION_REVERSE,
        LIMITS_WRITE,
        WEBHOOKS_READ,
        WEBHOOKS_WRITE,
        MERCHANTS_WRITE,
        CARDS_READ,
        CARDS_WRITE,
        CARDS_CHARGE,
        DEPOSITS_WRITE,
    ];
}
