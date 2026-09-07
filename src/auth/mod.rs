use std::{collections::HashMap, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::post,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

use crate::{
    AppError,
    auth::{extractor::AuthClaims, model::CredentialData},
    connector::app_state::{AppState, AppStateAuthentication},
    dcp::{
        holder::{self, HolderState},
        issuer::{self, IssuerState},
        oid4vci::{self, Oid4vciState},
        si_token::{DidResolver, ReplayCache, build_si_token},
        store::CredentialStore,
        sts::{self, StsState},
    },
    shared::KeyPair,
    store::Store,
};

pub(crate) mod extractor;
pub(crate) mod model;
mod token;

#[cfg(all(test, not(feature = "tck")))]
mod tests;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Everything needed to stand up the connector's DCP wallet.
pub(crate) struct WalletConfig {
    pub(crate) key_pair: KeyPair,
    pub(crate) local_did: String,
    pub(crate) kid: String,
    pub(crate) allowed_issuers: Vec<String>,
    pub(crate) resolver: Arc<dyn DidResolver>,
    pub(crate) client: Client,
    pub(crate) store: Arc<dyn CredentialStore>,
    pub(crate) base_address: String,
    pub(crate) credential_service_path: String,
    pub(crate) issuance_service_path: String,
    /// walt.id issuer base URL, used to redeem credential offers over OID4VCI.
    pub(crate) issuer_url: String,
    /// `Some((client_id, client_secret))` mounts the STS; `None` leaves it off.
    pub(crate) sts_credentials: Option<(String, String)>,
    /// Mount the native DCP issuance paths (self-issued credentials). Off when an
    /// external OID4VCI issuer is the credential source.
    pub(crate) dcp_issuance: bool,
}

/// The connector's identity: a native DCP wallet (holder + verifier, plus a retained
/// self-issuance path) behind one `did:web`.
///
/// Concrete on purpose — there is a single implementation, so there is no backend trait
/// and no downcast in the request path.
pub(crate) struct Authenticator {
    key_pair: KeyPair,
    local_did: String,
    kid: String,
    allowed_issuers: Vec<String>,
    resolver: Arc<dyn DidResolver>,
    replay: ReplayCache,
    client: Client,
    local_did_document: Value,
    holder: HolderState,
    issuer: IssuerState,
    /// `None` when no STS credentials are configured — the endpoint is then not mounted.
    sts: Option<StsState>,
    oid4vci: Oid4vciState,
    dcp_issuance: bool,
    credential_service_path: String,
    issuance_service_path: String,
}

impl Authenticator {
    pub(crate) fn new(config: WalletConfig) -> Self {
        let store = config.store;

        let holder = HolderState::new(
            config.key_pair.clone(),
            config.local_did.clone(),
            config.kid.clone(),
            config.client.clone(),
            config.resolver.clone(),
            store.clone(),
        );

        let issuer = IssuerState::new(
            config.key_pair.clone(),
            config.local_did.clone(),
            config.kid.clone(),
            config.client.clone(),
            config.resolver.clone(),
        );

        let sts = config
            .sts_credentials
            .map(|(client_id, client_secret)| {
                StsState::new(
                    config.key_pair.clone(),
                    config.local_did.clone(),
                    config.kid.clone(),
                    client_id,
                    client_secret,
                )
            });

        let oid4vci = Oid4vciState::new(
            config.key_pair.clone(),
            config.local_did.clone(),
            config.kid.clone(),
            config.client.clone(),
            store,
            config.issuer_url,
        );

        let base = config.base_address.trim_end_matches('/');
        let local_did_document = build_local_did_document(
            &config.local_did,
            &config.kid,
            serde_json::to_value(config.key_pair.public_jwk()).unwrap_or(Value::Null),
            &format!("{base}{}", config.credential_service_path),
            &format!("{base}{}", config.issuance_service_path),
        );

        Self {
            key_pair: config.key_pair,
            local_did: config.local_did,
            kid: config.kid,
            allowed_issuers: config.allowed_issuers,
            resolver: config.resolver,
            replay: ReplayCache::new(),
            client: config.client,
            local_did_document,
            holder,
            issuer,
            sts,
            oid4vci,
            dcp_issuance: config.dcp_issuance,
            credential_service_path: config.credential_service_path,
            issuance_service_path: config.issuance_service_path,
        }
    }

    /// Obtain a DSP access token from `remote_address` by presenting a Self-Issued ID Token.
    pub(crate) async fn get_token(
        &self,
        #[cfg_attr(feature = "tck", allow(unused))] client: &Client,
        #[cfg_attr(feature = "tck", allow(unused))] remote_address: &str,
        #[cfg_attr(feature = "tck", allow(unused))] did_web: String,
    ) -> anyhow::Result<String> {
        #[cfg(feature = "tck")]
        return Ok("".to_string());

        #[cfg_attr(feature = "tck", allow(unreachable_code))]
        let target_did = if did_web.is_empty() || did_web == self.local_did {
            crate::shared::derive_did_web(remote_address)?
        } else {
            did_web
        };

        let si_token = build_si_token(
            &self.key_pair,
            self.local_did.clone(),
            target_did,
            self.kid.clone(),
            None,
        )?;

        let trimmed_remote = remote_address.trim_end_matches('/');
        let response = client
            .post(format!("{trimmed_remote}/auth/token"))
            .bearer_auth(si_token)
            .send()
            .await?
            .error_for_status()?;

        let token_resp: TokenResponse = response.json().await?;
        Ok(token_resp.access_token)
    }

    /// Resolve a DID document; the connector's own is served from memory.
    pub(crate) async fn did_document(&self, _client: &Client, did: &str) -> anyhow::Result<Value> {
        if did == self.local_did {
            return Ok(self.local_did_document.clone());
        }

        Ok(serde_json::to_value(self.resolver.resolve(did).await?)?)
    }

    /// Map a presented `identity` credential onto a DSP access token.
    pub(crate) fn derive_access_token(
        &self,
        credentials: HashMap<String, CredentialData>,
        iss_did_web: String,
        sub_did_web: String,
    ) -> anyhow::Result<Option<String>> {
        let mut claims = AuthClaims::default();

        debug!("Presented credentials:\n{:?}", &credentials);
        let mut found = false;
        for (id, cd) in credentials {
            if id.as_str() == "identity"
                && let Ok(data) =
                    serde_json::from_value::<IdentityCredentialData>(cd.credential_data)
            {
                claims.data.insert("email".into(), data.email);
                claims.data.insert("country".into(), data.address.country);
                found = true;
            }
        }
        if !found {
            return Ok(None);
        }

        claims.data.insert("iss".into(), Value::String(iss_did_web));
        claims.data.insert("sub".into(), Value::String(sub_did_web));

        self.key_pair.encode(&claims).map(Some)
    }

    pub(crate) fn new_transfer_token(
        &self,
        pid: String,
        iss_did_web: String,
    ) -> anyhow::Result<String> {
        let mut claims = AuthClaims::default();
        claims.data.insert("sub".to_string(), Value::String(pid));
        claims
            .data
            .insert("iss".to_string(), Value::String(iss_did_web));

        self.key_pair.encode(&claims)
    }

    pub(crate) fn decode(&self, token: &str) -> anyhow::Result<AuthClaims> {
        self.key_pair.decode(token)
    }

    /// Credential service, issuance service and STS, all owned by the wallet.
    ///
    /// STS sits under `/api-internal` because nothing in the protocol calls it — it is a
    /// local token-minting affordance, not a peer-facing endpoint.
    fn service_routes(&self) -> Router {
        let mut router = Router::new().nest(
            &self.credential_service_path,
            holder::router(self.holder.clone(), self.dcp_issuance),
        );

        if self.dcp_issuance {
            router = router.nest(
                &self.issuance_service_path,
                issuer::router(self.issuer.clone()),
            );
        }

        if let Some(sts) = &self.sts {
            router = router.nest("/api-internal/sts", sts::router(sts.clone()));
        }

        router.nest(
            "/api-internal/credentials",
            oid4vci::router(self.oid4vci.clone()),
        )
    }
}

#[cfg(test)]
impl Authenticator {
    /// Build a wallet directly, without a config file.
    ///
    /// Goes through the single real constructor, so tests exercise the production path.
    pub(crate) fn for_test(
        key_pair: KeyPair,
        local_did: impl Into<String>,
        allowed_issuers: Vec<String>,
        resolver: Arc<dyn DidResolver>,
        store: Arc<dyn CredentialStore>,
    ) -> Self {
        let local_did = local_did.into();
        let authority = local_did.trim_start_matches("did:web:").replace("%3A", ":");

        Self::new(WalletConfig {
            kid: format!("{local_did}#keys-1"),
            key_pair,
            local_did,
            allowed_issuers,
            resolver,
            client: Client::builder().no_proxy().build().expect("test client"),
            store,
            base_address: format!("http://{authority}"),
            credential_service_path: "/api/credentials/v1".to_string(),
            issuance_service_path: "/api/issuance/v1".to_string(),
            issuer_url: String::new(),
            sts_credentials: None,
            dcp_issuance: true,
        })
    }

    pub(crate) fn credential_store(&self) -> Arc<dyn CredentialStore> {
        self.holder.store()
    }
}

fn build_local_did_document(
    local_did: &str,
    kid: &str,
    public_jwk: Value,
    credential_endpoint: &str,
    issuer_endpoint: &str,
) -> Value {
    json!({
        "id": local_did,
        "verificationMethod": [{
            "id": kid,
            "type": "JsonWebKey2020",
            "controller": local_did,
            "publicKeyJwk": public_jwk
        }],
        "authentication": [kid],
        "assertionMethod": [kid],
        "capabilityInvocation": [kid],
        "service": [
            {
                "id": format!("{local_did}#credential-service"),
                "type": "CredentialService",
                "serviceEndpoint": credential_endpoint
            },
            {
                "id": format!("{local_did}#issuer-service"),
                "type": "IssuerService",
                "serviceEndpoint": issuer_endpoint
            }
        ]
    })
}

#[derive(Debug, Deserialize)]
struct Address {
    country: Value,
}

#[derive(Debug, Deserialize)]
struct IdentityCredentialData {
    email: Value,
    address: Address,
}

pub(crate) fn router<T: Store>() -> Router<AppState<T>> {
    Router::new()
        .route("/token", post(token::auth_token))
        .merge({
            let r = Router::new();
            #[cfg(feature = "tck")]
            let r = r.route("/tck/get_token", post(tck::get_token));
            r
        })
}

/// The wallet's own service routes, lifted into the connector's state type.
pub(crate) fn service_routes<T: Store>(state: &AppState<T>) -> Router<AppState<T>> {
    state.authenticator.service_routes().with_state(())
}

pub(crate) async fn did(
    State(state): State<AppStateAuthentication>,
) -> Result<impl IntoResponse, AppError> {
    let did = state.participant_info.did_web()?;
    let did_document = state.authenticator.did_document(&state.client, &did).await?;
    Ok(Json(did_document))
}

#[cfg(feature = "tck")]
mod tck {
    use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
    use serde::Deserialize;

    use crate::{AppError, connector::app_state::AppStateAuthentication};

    #[derive(Debug, Deserialize)]
    pub(super) struct GetTokenRequest {
        remote_address: String,
    }

    pub(super) async fn get_token(
        State(state): State<AppStateAuthentication>,
        Json(request): Json<GetTokenRequest>,
    ) -> Result<impl IntoResponse, AppError> {
        let access_token = state
            .authenticator
            .get_token(
                &state.client,
                &request.remote_address,
                state.participant_info.did_web()?,
            )
            .await?;
        Ok((StatusCode::OK, access_token))
    }
}
