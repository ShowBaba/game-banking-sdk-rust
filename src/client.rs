use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::Arc;

use crate::error::Result;
use crate::transport::{
    api_path, check_amount, sealed, BoxFuture, ClientBuilder, Environment, FromInner, Inner, Op,
    Query, RequestOptions,
};
use crate::types::*;

/// Declares a resource: a handle on the client that carries per-call options.
macro_rules! resource {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone)]
        pub struct $name {
            pub(crate) t: std::sync::Arc<$crate::transport::Inner>,
            pub(crate) opts: $crate::transport::RequestOptions,
        }

        impl $name {
            pub(crate) fn new(t: &std::sync::Arc<$crate::transport::Inner>) -> Self {
                $name { t: t.clone(), opts: Default::default() }
            }

            /// Sets your own stable Idempotency-Key for the call. Retrying with
            /// the same key never moves money twice.
            pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
                self.opts.idempotency_key = Some(key.into());
                self
            }

            /// Overrides the client's retry count for the call.
            pub fn max_retries(mut self, n: u32) -> Self {
                self.opts.max_retries = Some(n);
                self
            }

            /// Sets every per-call option at once.
            pub fn with_options(mut self, opts: $crate::transport::RequestOptions) -> Self {
                self.opts = opts;
                self
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).field("options", &self.opts).finish()
            }
        }
    };
}
pub(crate) use resource;

type Fetch<T> = Box<dyn FnMut(Option<String>) -> BoxFuture<'static, Result<Page<T>>> + Send>;

/// Follows `next_cursor` through a paginated list, one item at a time.
///
/// ```no_run
/// # async fn run(gb: gamebanking::GameBank, account_id: &str) -> gamebanking::Result<()> {
/// let mut lines = gb.accounts().iterate_transactions(account_id, 50);
/// while let Some(line) = lines.next().await {
///     let line = line?;
///     println!("{} {}", line.direction, line.amount_display);
/// }
/// # Ok(()) }
/// ```
pub struct Pager<T> {
    fetch: Fetch<T>,
    buf: VecDeque<T>,
    cursor: Option<String>,
    done: bool,
}

impl<T> Pager<T> {
    pub(crate) fn new(
        fetch: impl FnMut(Option<String>) -> BoxFuture<'static, Result<Page<T>>> + Send + 'static,
    ) -> Self {
        Pager {
            fetch: Box::new(fetch),
            buf: VecDeque::new(),
            cursor: None,
            done: false,
        }
    }

    /// The next item, fetching the next page when needed; `None` at the end.
    /// After an error the pager stops.
    #[allow(clippy::should_implement_trait)]
    pub async fn next(&mut self) -> Option<Result<T>> {
        loop {
            if let Some(item) = self.buf.pop_front() {
                return Some(Ok(item));
            }
            if self.done {
                return None;
            }
            match (self.fetch)(self.cursor.take()).await {
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
                Ok(page) => {
                    self.buf.extend(page.data);
                    match page.next_cursor.filter(|c| !c.is_empty()) {
                        Some(c) => self.cursor = Some(c),
                        None => self.done = true,
                    }
                }
            }
        }
    }

    /// Reads every remaining item.
    pub async fn collect(mut self) -> Result<Vec<T>> {
        let mut out = Vec::new();
        while let Some(item) = self.next().await {
            out.push(item?);
        }
        Ok(out)
    }
}

impl<T> fmt::Debug for Pager<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pager")
            .field("buffered", &self.buf.len())
            .field("done", &self.done)
            .finish()
    }
}

/// The runtime client for game servers, authenticated with a game API key.
/// Cheap to clone; clones share one connection pool.
///
/// ```no_run
/// # async fn run() -> gamebanking::Result<()> {
/// use gamebanking::{GameBank, TransferParams};
/// let gb = GameBank::builder().api_key("gbe_test_sk_…").build()?; // talks to https://api.gamebank.dev
/// let sent = gb
///     .transfers()
///     .create(&TransferParams {
///         source_account_id: "…".into(),
///         destination_account_number: Some("5915945174".into()),
///         amount_minor: 1_000,
///         ..Default::default()
///     })
///     .await?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct GameBank {
    pub(crate) t: Arc<Inner>,
}

impl sealed::Sealed for GameBank {}

impl FromInner for GameBank {
    fn from_inner(inner: Arc<Inner>) -> Self {
        GameBank { t: inner }
    }
}

impl fmt::Debug for GameBank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameBank")
            .field("environment", &self.t.environment)
            .finish_non_exhaustive()
    }
}

impl GameBank {
    /// Starts building a client.
    pub fn builder() -> ClientBuilder<GameBank> {
        ClientBuilder::new()
    }

    /// A client configured from `GBE_API_KEY` (and `GBE_BASE_URL`, if set).
    pub fn from_env() -> Result<GameBank> {
        Self::builder().build()
    }

    /// Sandbox or production, read from the API key's prefix.
    pub fn environment(&self) -> Option<Environment> {
        self.t.environment
    }

    /// The engine this client talks to.
    pub fn base_url(&self) -> &str {
        &self.t.base_url
    }

    pub fn players(&self) -> Players {
        Players::new(&self.t)
    }

    pub fn accounts(&self) -> Accounts {
        Accounts::new(&self.t)
    }

    pub fn funding(&self) -> Funding {
        Funding::new(&self.t)
    }

    pub fn transfers(&self) -> Transfers {
        Transfers::new(&self.t)
    }

    pub fn holds(&self) -> Holds {
        Holds::new(&self.t)
    }

    pub fn transactions(&self) -> Transactions {
        Transactions::new(&self.t)
    }

    pub fn banks(&self) -> Banks {
        Banks::new(&self.t)
    }

    pub fn fees(&self) -> Fees {
        Fees::new(&self.t)
    }

    pub fn currencies(&self) -> Currencies {
        Currencies::new(&self.t)
    }

    pub fn clock(&self) -> ClockResource {
        ClockResource::new(&self.t)
    }

    pub fn webhooks(&self) -> crate::webhooks::Webhooks {
        crate::webhooks::Webhooks::new(&self.t)
    }

    pub fn merchants(&self) -> crate::cards::Merchants {
        crate::cards::Merchants::new(&self.t)
    }

    pub fn card_programs(&self) -> crate::cards::CardPrograms {
        crate::cards::CardPrograms::new(&self.t)
    }

    pub fn cards(&self) -> crate::cards::Cards {
        crate::cards::Cards::new(&self.t)
    }

    /// Authorises, captures, releases and refunds card payments.
    pub fn card_authorizations(&self) -> crate::cards::CardAuthorizations {
        crate::cards::CardAuthorizations::new(&self.t)
    }

    pub fn deposit_products(&self) -> crate::savings::DepositProducts {
        crate::savings::DepositProducts::new(&self.t)
    }

    pub fn fixed_deposits(&self) -> crate::savings::FixedDeposits {
        crate::savings::FixedDeposits::new(&self.t)
    }

    /// Confirms a recipient before a transfer: only the account name and status.
    pub async fn name_enquiry(&self, bank_code: &str, account_number: &str) -> Result<NameEnquiry> {
        let q = Query::new()
            .set("bank_code", bank_code)
            .set("account_number", account_number);
        self.t
            .call(
                Op::get("/v1/name-enquiry".into()).query(q),
                &RequestOptions::default(),
            )
            .await
    }

    /// Compares stored balances with the ledger for this game and environment.
    pub async fn reconcile(&self) -> Result<ReconciliationReport> {
        self.t
            .call(
                Op::get("/v1/reconciliation".into()),
                &RequestOptions::default(),
            )
            .await
    }
}

resource!(Players);

impl Players {
    /// Maps your player ID to an engine player; repeat calls return the same player.
    pub async fn upsert(&self, p: &UpsertPlayerParams) -> Result<Player> {
        self.t
            .call(
                Op::post("/v1/players".into()).json(p).retry_safe(),
                &self.opts,
            )
            .await
    }

    pub async fn get(&self, player_id: &str) -> Result<Player> {
        self.t
            .call(Op::get(api_path!("/v1/players/{}", player_id)), &self.opts)
            .await
    }

    /// Looks a player up by your own player ID.
    pub async fn find_by_external_id(&self, external_id: &str) -> Result<Player> {
        let q = Query::new().set("external_player_id", external_id);
        self.t
            .call(Op::get("/v1/players".into()).query(q), &self.opts)
            .await
    }
}

resource!(Accounts);

impl Accounts {
    /// Opens a player, merchant or savings account.
    pub async fn open(&self, p: &OpenAccountParams) -> Result<Account> {
        self.t
            .call(Op::post("/v1/accounts".into()).json(p), &self.opts)
            .await
    }

    pub async fn get(&self, account_id: &str) -> Result<Account> {
        self.t
            .call(
                Op::get(api_path!("/v1/accounts/{}", account_id)),
                &self.opts,
            )
            .await
    }

    pub async fn list(&self, p: &ListAccountsParams) -> Result<Vec<Account>> {
        let q = Query::new()
            .opt("bank_id", p.bank_id.as_ref())
            .opt("player_id", p.player_id.as_ref())
            .opt("merchant_id", p.merchant_id.as_ref())
            .opt("include_system", p.include_system.then_some("true"))
            .opt("limit", p.limit);
        let out: List<Account> = self
            .t
            .call(Op::get("/v1/accounts".into()).query(q), &self.opts)
            .await?;
        Ok(out.data)
    }

    /// Freezes (no debits), unfreezes (`active`) or closes an account. Closing needs a zero balance.
    pub async fn update_status(
        &self,
        account_id: &str,
        p: &UpdateAccountStatusParams,
    ) -> Result<Account> {
        self.t
            .call(
                Op::patch(api_path!("/v1/accounts/{}", account_id)).json(p),
                &self.opts,
            )
            .await
    }

    pub async fn balance(&self, account_id: &str) -> Result<Balance> {
        self.t
            .call(
                Op::get(api_path!("/v1/accounts/{}/balance", account_id)),
                &self.opts,
            )
            .await
    }

    /// One page of history, newest first. Pass `next_cursor` back as `cursor`.
    pub async fn transactions(
        &self,
        account_id: &str,
        limit: Option<u32>,
        cursor: Option<&str>,
    ) -> Result<HistoryPage> {
        let q = Query::new().opt("limit", limit).opt("cursor", cursor);
        self.t
            .call(
                Op::get(api_path!("/v1/accounts/{}/transactions", account_id)).query(q),
                &self.opts,
            )
            .await
    }

    /// Every history line, newest first, fetching pages of `page_size` as needed.
    pub fn iterate_transactions(&self, account_id: &str, page_size: u32) -> Pager<HistoryItem> {
        let this = self.clone();
        let id = account_id.to_string();
        Pager::new(move |cursor| {
            let (this, id) = (this.clone(), id.clone());
            Box::pin(async move {
                this.transactions(&id, Some(page_size), cursor.as_deref())
                    .await
            })
        })
    }

    /// The account's holds; `status` `None` for all.
    pub async fn holds(&self, account_id: &str, status: Option<&str>) -> Result<Vec<Hold>> {
        let q = Query::new().opt("status", status);
        let out: List<Hold> = self
            .t
            .call(
                Op::get(api_path!("/v1/accounts/{}/holds", account_id)).query(q),
                &self.opts,
            )
            .await?;
        Ok(out.data)
    }

    /// A savings or fixed-deposit account's rate, accrued interest and payouts.
    pub async fn interest(&self, account_id: &str) -> Result<crate::savings::AccountInterest> {
        self.t
            .call(
                Op::get(api_path!("/v1/accounts/{}/interest", account_id)),
                &self.opts,
            )
            .await
    }
}

resource!(Funding);

impl Funding {
    /// Credits a player account from the bank's issuance account. Needs `funding:create`.
    pub async fn create(&self, p: &FundingParams) -> Result<FundingResult> {
        check_amount(p.amount_minor, true)?;
        self.t
            .call(
                Op::post("/v1/funding".into()).json(p).idempotent(),
                &self.opts,
            )
            .await
    }
}

resource!(Transfers);

impl Transfers {
    /// Moves money between two customer accounts in the same bank, atomically.
    pub async fn create(&self, p: &TransferParams) -> Result<TransferResult> {
        check_amount(p.amount_minor, true)?;
        self.t
            .call(
                Op::post("/v1/transfers".into()).json(p).idempotent(),
                &self.opts,
            )
            .await
    }
}

resource!(Holds);

impl Holds {
    /// Reserves funds: the available balance drops, the posted balance does not.
    pub async fn place(&self, p: &PlaceHoldParams) -> Result<HoldResult> {
        check_amount(p.amount_minor, true)?;
        self.t
            .call(
                Op::post("/v1/holds".into()).json(p).idempotent(),
                &self.opts,
            )
            .await
    }

    pub async fn get(&self, hold_id: &str) -> Result<Hold> {
        self.t
            .call(Op::get(api_path!("/v1/holds/{}", hold_id)), &self.opts)
            .await
    }

    /// Posts all or part of a hold as a transfer and releases the rest.
    pub async fn capture(&self, hold_id: &str, p: &CaptureHoldParams) -> Result<CaptureResult> {
        check_amount(p.amount_minor.unwrap_or(0), false)?;
        self.t
            .call(
                Op::post(api_path!("/v1/holds/{}/capture", hold_id))
                    .json(p)
                    .idempotent(),
                &self.opts,
            )
            .await
    }

    /// Cancels a hold and restores the available balance.
    pub async fn release(&self, hold_id: &str, reason: Option<&str>) -> Result<HoldResult> {
        let body = reason_body(reason);
        self.t
            .call(
                Op::post(api_path!("/v1/holds/{}/release", hold_id))
                    .json(&body)
                    .idempotent(),
                &self.opts,
            )
            .await
    }
}

pub(crate) fn reason_body(reason: Option<&str>) -> BTreeMap<&'static str, &str> {
    reason.map(|r| ("reason", r)).into_iter().collect()
}

resource!(Transactions);

impl Transactions {
    pub async fn get(&self, transaction_id: &str) -> Result<Transaction> {
        self.t
            .call(
                Op::get(api_path!("/v1/transactions/{}", transaction_id)),
                &self.opts,
            )
            .await
    }

    /// Posts a compensating transaction. Needs `transaction:reverse`.
    pub async fn reverse(&self, transaction_id: &str, p: &ReverseParams) -> Result<Transaction> {
        self.t
            .call(
                Op::post(api_path!("/v1/transactions/{}/reverse", transaction_id))
                    .json(p)
                    .idempotent(),
                &self.opts,
            )
            .await
    }
}

resource!(Banks);

impl Banks {
    /// Creates a bank with its own system ledger accounts.
    pub async fn create(&self, p: &CreateBankParams) -> Result<Bank> {
        self.t
            .call(Op::post("/v1/banks".into()).json(p), &self.opts)
            .await
    }

    pub async fn list(&self) -> Result<Vec<Bank>> {
        let out: List<Bank> = self.t.call(Op::get("/v1/banks".into()), &self.opts).await?;
        Ok(out.data)
    }

    pub async fn get(&self, bank_id: &str) -> Result<Bank> {
        self.t
            .call(Op::get(api_path!("/v1/banks/{}", bank_id)), &self.opts)
            .await
    }

    pub async fn update(&self, bank_id: &str, p: &UpdateBankParams) -> Result<Bank> {
        self.t
            .call(
                Op::patch(api_path!("/v1/banks/{}", bank_id)).json(p),
                &self.opts,
            )
            .await
    }

    pub async fn limits(&self, bank_id: &str) -> Result<Limits> {
        self.t
            .call(
                Op::get(api_path!("/v1/banks/{}/limits", bank_id)),
                &self.opts,
            )
            .await
    }

    /// Sets caps per [`limit_scope`]; `None` removes that scope's limits. Needs `limits:write`.
    pub async fn set_limits(
        &self,
        bank_id: &str,
        limits: &BTreeMap<String, Option<LimitPolicy>>,
    ) -> Result<Limits> {
        self.t
            .call(
                Op::put(api_path!("/v1/banks/{}/limits", bank_id)).json(limits),
                &self.opts,
            )
            .await
    }

    /// The bank's own ledger accounts: fee income, levies held, interest and fixed deposits.
    pub async fn books(&self, bank_id: &str) -> Result<BankBooks> {
        self.t
            .call(
                Op::get(api_path!("/v1/banks/{}/books", bank_id)),
                &self.opts,
            )
            .await
    }
}

resource!(
    /// A bank's fees and levies, and quotes.
    Fees
);

impl Fees {
    /// A bank's fee rules, active and inactive, each with its current `state`.
    pub async fn list(&self, bank_id: &str) -> Result<Vec<FeeRule>> {
        let out: List<FeeRule> = self
            .t
            .call(Op::get(api_path!("/v1/banks/{}/fees", bank_id)), &self.opts)
            .await?;
        Ok(out.data)
    }

    pub async fn get(&self, bank_id: &str, fee_id: &str) -> Result<FeeRule> {
        self.t
            .call(
                Op::get(api_path!("/v1/banks/{}/fees/{}", bank_id, fee_id)),
                &self.opts,
            )
            .await
    }

    /// Adds a fee or levy. Needs `bank:write`.
    pub async fn create(&self, bank_id: &str, p: &CreateFeeRuleParams) -> Result<FeeRule> {
        self.t
            .call(
                Op::post(api_path!("/v1/banks/{}/fees", bank_id)).json(p),
                &self.opts,
            )
            .await
    }

    /// Changes, pauses (`status: inactive`) or resumes a fee and bumps its version. Needs `bank:write`.
    pub async fn update(
        &self,
        bank_id: &str,
        fee_id: &str,
        p: &UpdateFeeRuleParams,
    ) -> Result<FeeRule> {
        self.t
            .call(
                Op::patch(api_path!("/v1/banks/{}/fees/{}", bank_id, fee_id)).json(p),
                &self.opts,
            )
            .await
    }

    /// What a payment would cost now. Show it to the player, then pass
    /// `fees_total_minor` as `max_fee_minor` on the payment.
    pub async fn quote(&self, p: &QuoteFeesParams) -> Result<FeeQuote> {
        check_amount(p.amount_minor, true)?;
        self.t
            .call(Op::post("/v1/fees/quote".into()).json(p), &self.opts)
            .await
    }
}

resource!(Currencies);

impl Currencies {
    /// Standard ISO currencies plus this game's custom units.
    pub async fn list(&self) -> Result<Vec<Currency>> {
        let out: List<Currency> = self
            .t
            .call(Op::get("/v1/currencies".into()), &self.opts)
            .await?;
        Ok(out.data)
    }

    /// Defines a custom virtual unit, e.g. `GOLD` with exponent 0.
    pub async fn create(&self, p: &CreateCurrencyParams) -> Result<Currency> {
        self.t
            .call(Op::post("/v1/currencies".into()).json(p), &self.opts)
            .await
    }
}

resource!(
    /// The game clock.
    ClockResource
);

impl ClockResource {
    pub async fn get(&self) -> Result<Clock> {
        self.t.call(Op::get("/v1/clock".into()), &self.opts).await
    }

    /// Changes mode, speed or time zone, or jumps forward. Needs `limits:write`.
    pub async fn configure(&self, p: &ConfigureClockParams) -> Result<Clock> {
        self.t
            .call(Op::put("/v1/clock".into()).json(p), &self.opts)
            .await
    }

    /// Moves a manual clock forward and runs the jobs that became due.
    pub async fn advance(&self, p: &AdvanceClockParams) -> Result<AdvanceClockResult> {
        self.t
            .call(Op::post("/v1/clock/advance".into()).json(p), &self.opts)
            .await
    }
}
