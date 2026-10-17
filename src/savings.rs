//! Savings products, savings accounts and fixed deposits.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::client::resource;
use crate::error::Result;
use crate::transport::{api_path, check_amount, Op, Query};
use crate::types::{nullable, Balance, List, Timestamp, Transaction};

/// An annual rate from a balance upwards (300 = 3%).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepositTier {
    pub from_minor: i64,
    pub rate_bps: i32,
}

/// A fixed-deposit term on offer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepositTerm {
    pub days: i32,
    pub rate_bps: i32,
}

/// Product kinds.
pub mod product_kind {
    pub const SAVINGS: &str = "savings";
    pub const FIXED_DEPOSIT: &str = "fixed_deposit";
}

/// What happens to a fixed deposit at maturity.
pub mod maturity {
    /// Principal and interest go to the payout account.
    pub const PAYOUT: &str = "payout";
    /// The interest is paid out; the principal starts a new term at today's rate.
    pub const ROLLOVER_PRINCIPAL: &str = "rollover_principal";
    /// Principal plus interest starts a new term.
    pub const ROLLOVER_ALL: &str = "rollover_all";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DepositProduct {
    pub id: String,
    pub bank_id: String,
    pub code: String,
    pub name: String,
    /// `savings` or `fixed_deposit`.
    pub kind: String,
    #[serde(deserialize_with = "nullable")]
    pub rate_tiers: Vec<DepositTier>,
    /// `whole_balance` or `banded`.
    pub tier_method: String,
    #[serde(deserialize_with = "nullable")]
    pub terms: Vec<DepositTerm>,
    /// `act_365` or `act_360`.
    pub day_count: String,
    /// `end_of_day` or `minimum_daily`.
    pub accrual_basis: String,
    /// `monthly` or `quarterly`.
    pub payout_frequency: String,
    pub withholding_tax_bps: i32,
    pub min_opening_minor: i64,
    pub min_balance_minor: i64,
    pub max_balance_minor: Option<i64>,
    pub withdrawals_per_month: Option<i32>,
    /// `block`, `forfeit_interest` or `fee`.
    pub excess_withdrawal_action: String,
    pub early_forfeit_bps: i32,
    pub early_penalty_fixed_minor: i64,
    pub early_penalty_bps: i32,
    pub default_maturity_instruction: String,
    pub status: String,
    pub version: i32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Creates or updates a product; `None` fields are left to the engine's
/// defaults (or unchanged). `code` and `kind` are set on create only.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct DepositProductParams {
    pub code: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub rate_tiers: Option<Vec<DepositTier>>,
    pub tier_method: Option<String>,
    pub terms: Option<Vec<DepositTerm>>,
    pub day_count: Option<String>,
    pub accrual_basis: Option<String>,
    pub payout_frequency: Option<String>,
    pub withholding_tax_bps: Option<i32>,
    pub min_opening_minor: Option<i64>,
    pub min_balance_minor: Option<i64>,
    pub max_balance_minor: Option<i64>,
    pub withdrawals_per_month: Option<i32>,
    pub excess_withdrawal_action: Option<String>,
    pub early_forfeit_bps: Option<i32>,
    pub early_penalty_fixed_minor: Option<i64>,
    pub early_penalty_bps: Option<i32>,
    pub default_maturity_instruction: Option<String>,
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clear: Vec<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct DepositQuoteParams {
    pub product_id: String,
    pub amount_minor: i64,
    /// Savings: the days to project (default 30).
    pub days: Option<i32>,
    /// Fixed deposits: the term.
    pub term_days: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DepositQuote {
    pub product_id: String,
    pub amount_minor: i64,
    pub days: i32,
    pub rate_bps: i32,
    pub gross_interest_minor: i64,
    pub tax_minor: i64,
    pub net_interest_minor: i64,
    pub maturity_amount_minor: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InterestPayout {
    pub transaction_id: String,
    pub gross_minor: i64,
    pub narration: String,
    pub game_time: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountInterest {
    pub account_id: String,
    pub product_id: String,
    pub product_name: String,
    pub payout_frequency: String,
    /// The rate the current balance earns (a fixed deposit's locked rate).
    pub rate_bps: i32,
    pub accrued_interest_minor: i64,
    pub accrued_through: String,
    pub next_payout_day: String,
    pub forfeit_month: Option<String>,
    pub withdrawals_this_month: i32,
    pub withdrawals_per_month: Option<i32>,
    #[serde(deserialize_with = "nullable")]
    pub payouts: Vec<InterestPayout>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FixedDeposit {
    pub id: String,
    pub bank_id: String,
    pub account_id: String,
    pub account_number: String,
    pub player_id: String,
    pub product_id: String,
    pub product_name: String,
    pub source_account_id: String,
    pub payout_account_id: String,
    pub principal_minor: i64,
    pub rate_bps: i32,
    pub term_days: i32,
    pub start_day: String,
    pub maturity_day: String,
    pub maturity_instruction: String,
    /// `active`, `matured` or `broken`.
    pub status: String,
    pub terms_completed: i32,
    pub interest_paid_minor: i64,
    pub accrued_interest_minor: i64,
    pub balance_minor: i64,
    pub created_at: Timestamp,
    pub closed_at: Option<Timestamp>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize)]
pub struct OpenFixedDepositParams {
    pub source_account_id: String,
    pub product_id: String,
    pub term_days: i32,
    pub amount_minor: i64,
    /// A [`maturity`] instruction; defaults to the product's.
    pub maturity_instruction: Option<String>,
    /// Defaults to the source account.
    pub payout_account_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenFixedDepositResult {
    pub fixed_deposit: FixedDeposit,
    pub transaction: Transaction,
    pub source_balance: Balance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BreakQuote {
    pub fixed_deposit_id: String,
    pub principal_minor: i64,
    pub accrued_interest_minor: i64,
    pub forfeited_interest_minor: i64,
    pub interest_paid_minor: i64,
    pub tax_minor: i64,
    pub penalty_minor: i64,
    pub payout_minor: i64,
    pub payout_account_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BreakFixedDepositResult {
    pub fixed_deposit: FixedDeposit,
    pub quote: BreakQuote,
    pub transaction: Transaction,
}

#[derive(Debug, Clone, Default)]
pub struct ListFixedDepositsParams {
    pub player_id: Option<String>,
    pub source_account_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
}

resource!(
    /// A bank's savings and fixed-deposit products.
    DepositProducts
);

impl DepositProducts {
    pub async fn list(&self, bank_id: &str) -> Result<Vec<DepositProduct>> {
        let out: List<DepositProduct> = self
            .t
            .call(
                Op::get(api_path!("/v1/banks/{}/deposit-products", bank_id)),
                &self.opts,
            )
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, bank_id: &str, product_id: &str) -> Result<DepositProduct> {
        self.t
            .call(
                Op::get(api_path!(
                    "/v1/banks/{}/deposit-products/{}",
                    bank_id,
                    product_id
                )),
                &self.opts,
            )
            .await
    }

    /// Adds a product. Needs `bank:write`; turn on the bank's `savings` flag to open accounts.
    pub async fn create(&self, bank_id: &str, p: &DepositProductParams) -> Result<DepositProduct> {
        self.t
            .call(
                Op::post(api_path!("/v1/banks/{}/deposit-products", bank_id)).json(p),
                &self.opts,
            )
            .await
    }

    /// Changes a product and bumps its version.
    pub async fn update(
        &self,
        bank_id: &str,
        product_id: &str,
        p: &DepositProductParams,
    ) -> Result<DepositProduct> {
        self.t
            .call(
                Op::patch(api_path!(
                    "/v1/banks/{}/deposit-products/{}",
                    bank_id,
                    product_id
                ))
                .json(p),
                &self.opts,
            )
            .await
    }

    /// Projects the interest an amount would earn.
    pub async fn quote(&self, p: &DepositQuoteParams) -> Result<DepositQuote> {
        check_amount(p.amount_minor, true)?;
        self.t
            .call(
                Op::post("/v1/deposit-products/quote".into()).json(p),
                &self.opts,
            )
            .await
    }
}

resource!(
    /// Books, changes and breaks fixed deposits.
    FixedDeposits
);

impl FixedDeposits {
    /// Books a fixed deposit at the term's current rate. Needs `deposits:write`.
    pub async fn open(&self, p: &OpenFixedDepositParams) -> Result<OpenFixedDepositResult> {
        check_amount(p.amount_minor, true)?;
        self.t
            .call(
                Op::post("/v1/fixed-deposits".into()).json(p).idempotent(),
                &self.opts,
            )
            .await
    }

    pub async fn list(&self, p: &ListFixedDepositsParams) -> Result<Vec<FixedDeposit>> {
        let q = Query::new()
            .opt("player_id", p.player_id.as_ref())
            .opt("source_account_id", p.source_account_id.as_ref())
            .opt("status", p.status.as_ref())
            .opt("limit", p.limit);
        let out: List<FixedDeposit> = self
            .t
            .call(Op::get("/v1/fixed-deposits".into()).query(q), &self.opts)
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, deposit_id: &str) -> Result<FixedDeposit> {
        self.t
            .call(
                Op::get(api_path!("/v1/fixed-deposits/{}", deposit_id)),
                &self.opts,
            )
            .await
    }

    /// Changes what happens at maturity (a [`maturity`] instruction).
    pub async fn update(
        &self,
        deposit_id: &str,
        maturity_instruction: &str,
    ) -> Result<FixedDeposit> {
        let body =
            std::collections::BTreeMap::from([("maturity_instruction", maturity_instruction)]);
        self.t
            .call(
                Op::patch(api_path!("/v1/fixed-deposits/{}", deposit_id)).json(&body),
                &self.opts,
            )
            .await
    }

    /// What breaking the deposit early pays now.
    pub async fn break_quote(&self, deposit_id: &str) -> Result<BreakQuote> {
        self.t
            .call(
                Op::get(api_path!("/v1/fixed-deposits/{}/break-quote", deposit_id)),
                &self.opts,
            )
            .await
    }

    /// Ends the deposit early. Pass the quote's `penalty_minor` as
    /// `max_penalty_minor` (or `None` for no cap); if the penalty has since
    /// risen the call fails with [`ErrorKind::PenaltyChanged`](crate::ErrorKind::PenaltyChanged).
    pub async fn break_deposit(
        &self,
        deposit_id: &str,
        max_penalty_minor: Option<i64>,
    ) -> Result<BreakFixedDepositResult> {
        let mut body = serde_json::Map::new();
        if let Some(m) = max_penalty_minor {
            body.insert("max_penalty_minor".into(), m.into());
        }
        self.t
            .call(
                Op::post(api_path!("/v1/fixed-deposits/{}/break", deposit_id))
                    .json(&body)
                    .idempotent(),
                &self.opts,
            )
            .await
    }
}
