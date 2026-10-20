use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::Deserialize;

use crate::client::{resource, GameBank};
use crate::error::Result;
use crate::transport::{
    api_path, sealed, ClientBuilder, FromInner, Inner, Op, Query, RequestOptions,
};
use crate::types::*;

/// The setup and CI client, authenticated with a developer credential
/// (`gbe_dev_…`) or a sign-in session token.
#[derive(Clone)]
pub struct GameBankManagement {
    t: Arc<Inner>,
}

impl sealed::Sealed for GameBankManagement {}

impl FromInner for GameBankManagement {
    fn from_inner(inner: Arc<Inner>) -> Self {
        GameBankManagement { t: inner }
    }
}

impl fmt::Debug for GameBankManagement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameBankManagement").finish_non_exhaustive()
    }
}

/// Filters for [`GameBankManagement::audit_logs`].
#[derive(Debug, Clone, Default)]
pub struct AuditLogParams {
    pub environment: Option<String>,
    pub limit: Option<u32>,
    pub before: Option<Timestamp>,
}

impl GameBankManagement {
    /// Starts building a management client.
    pub fn builder() -> ClientBuilder<GameBankManagement> {
        ClientBuilder::new()
    }

    /// A management client configured from `GBE_API_KEY` (and `GBE_BASE_URL`, if set).
    pub fn from_env() -> Result<GameBankManagement> {
        Self::builder().build()
    }

    /// The engine this client talks to.
    pub fn base_url(&self) -> &str {
        &self.t.base_url
    }

    pub fn games(&self) -> Games {
        Games::new(&self.t)
    }

    pub fn api_keys(&self) -> ApiKeys {
        ApiKeys::new(&self.t)
    }

    pub fn credentials(&self) -> DeveloperCredentials {
        DeveloperCredentials::new(&self.t)
    }

    /// The developer this credential belongs to.
    pub async fn me(&self) -> Result<Developer> {
        self.t
            .call(Op::get("/v1/developer".into()), &RequestOptions::default())
            .await
    }

    /// A game's audit log, newest first.
    pub async fn audit_logs(&self, game_id: &str, p: &AuditLogParams) -> Result<Vec<AuditLog>> {
        let q = Query::new()
            .opt("environment", p.environment.as_ref())
            .opt("limit", p.limit)
            .opt(
                "before",
                p.before
                    .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)),
            );
        let out: List<AuditLog> = self
            .t
            .call(
                Op::get(api_path!("/v1/games/{}/audit-logs", game_id)).query(q),
                &RequestOptions::default(),
            )
            .await?;
        Ok(out.data)
    }

    /// A [`GameBank`] acting on one of your games with your developer
    /// credential, to configure banks, currencies, limits and the clock.
    /// Moving money still needs a game API key.
    pub fn game(&self, game_id: &str, environment: &str) -> GameBank {
        GameBank {
            t: Arc::new(
                self.t
                    .with_headers(&[("X-Game-Id", game_id), ("X-Environment", environment)]),
            ),
        }
    }
}

resource!(Games);

impl Games {
    /// Registers a game; `slug` `None` derives it from the name.
    pub async fn create(&self, name: &str, slug: Option<&str>) -> Result<Game> {
        let mut body = BTreeMap::from([("name", name)]);
        if let Some(s) = slug {
            body.insert("slug", s);
        }
        self.t
            .call(Op::post("/v1/games".into()).json(&body), &self.opts)
            .await
    }

    pub async fn list(&self) -> Result<Vec<Game>> {
        let out: List<Game> = self.t.call(Op::get("/v1/games".into()), &self.opts).await?;
        Ok(out.data)
    }

    pub async fn get(&self, game_id: &str) -> Result<Game> {
        self.t
            .call(Op::get(api_path!("/v1/games/{}", game_id)), &self.opts)
            .await
    }

    /// Renames a game, changes its slug, or suspends or reactivates it.
    pub async fn update(&self, game_id: &str, p: &UpdateGameParams) -> Result<Game> {
        self.t
            .call(
                Op::patch(api_path!("/v1/games/{}", game_id)).json(p),
                &self.opts,
            )
            .await
    }

    /// Suspends or reactivates a game (`active` or `suspended`).
    pub async fn set_status(&self, game_id: &str, status: &str) -> Result<Game> {
        self.update(
            game_id,
            &UpdateGameParams {
                status: Some(status.into()),
                ..Default::default()
            },
        )
        .await
    }

    /// Deletes a game permanently: its API keys are revoked at once and it
    /// leaves the developer API. Ledger and audit history are kept.
    pub async fn delete(&self, game_id: &str) -> Result<()> {
        self.t
            .call_empty(Op::delete(api_path!("/v1/games/{}", game_id)), &self.opts)
            .await
    }
}

resource!(ApiKeys);

#[derive(Deserialize)]
struct IssuedKey {
    api_key: ApiKey,
}

impl ApiKeys {
    /// Creates a game API key. Its `secret` is in the result once: store it on your servers.
    pub async fn issue(&self, game_id: &str, p: &IssueApiKeyParams) -> Result<ApiKey> {
        let out: IssuedKey = self
            .t
            .call(
                Op::post(api_path!("/v1/games/{}/api-keys", game_id)).json(p),
                &self.opts,
            )
            .await?;
        Ok(out.api_key)
    }

    pub async fn list(&self, game_id: &str) -> Result<Vec<ApiKey>> {
        let out: List<ApiKey> = self
            .t
            .call(
                Op::get(api_path!("/v1/games/{}/api-keys", game_id)),
                &self.opts,
            )
            .await?;
        Ok(out.data)
    }

    /// Revokes a key immediately.
    pub async fn revoke(&self, game_id: &str, key_id: &str) -> Result<()> {
        self.t
            .call_empty(
                Op::delete(api_path!("/v1/games/{}/api-keys/{}", game_id, key_id)),
                &self.opts,
            )
            .await
    }
}

resource!(DeveloperCredentials);

impl DeveloperCredentials {
    pub async fn list(&self) -> Result<Vec<DeveloperCredential>> {
        let out: List<DeveloperCredential> = self
            .t
            .call(Op::get("/v1/developer/credentials".into()), &self.opts)
            .await?;
        Ok(out.data)
    }

    /// Creates a long-lived developer credential for scripts and CI.
    pub async fn issue(
        &self,
        label: &str,
        expires_at: Option<Timestamp>,
    ) -> Result<DeveloperCredential> {
        let mut body = serde_json::Map::new();
        body.insert("label".into(), label.into());
        if let Some(t) = expires_at {
            body.insert(
                "expires_at".into(),
                t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true).into(),
            );
        }
        self.t
            .call(
                Op::post("/v1/developer/credentials".into()).json(&body),
                &self.opts,
            )
            .await
    }

    pub async fn revoke(&self, credential_id: &str) -> Result<()> {
        self.t
            .call_empty(
                Op::delete(api_path!("/v1/developer/credentials/{}", credential_id)),
                &self.opts,
            )
            .await
    }
}
