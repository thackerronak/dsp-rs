use std::{any::Any, collections::HashMap, sync::Arc};

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use reqwest::Client;
use serde_json::{Value, json};

use crate::{
    auth::{Authenticator, backend::AuthBackend, model::CredentialData},
    connector::app_state::{AppStateAuthentication, KeyPair},
    dcp::{
        bearer_token,
        holder::{self, HolderState},
        issuer::{self, IssuerState},
        si_token::{DidResolver, ReplayCache, build_si_token, validate_si_token},
        sts::{self, StsState},
        verifier::{query_peer_presentation, validate_vc, validate_vp},
    },
    store::credential_store::CredentialStore,
};

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
}

pub(crate) struct NativeBackend {
    pub(crate) key_pair: KeyPair,
    pub(crate) local_did: String,
    pub(crate) kid: String,
    pub(crate) allowed_issuers: Vec<String>,
    pub(crate) resolver: Arc<dyn DidResolver>,
    pub(crate) replay: ReplayCache,
    pub(crate) client: Client,

    local_did_document: Option<Value>,
    holder_state: Option<HolderState>,
    issuer_state: Option<IssuerState>,
    sts_state: Option<StsState>,
    credential_service_path: String,
    issuance_service_path: String,
}

pub(crate) struct NativeWalletConfig {
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
    pub(crate) sts_client_id: String,
    pub(crate) sts_client_secret: String,
}

impl NativeBackend {
    #[allow(dead_code)]
    pub(crate) fn new(
        key_pair: KeyPair,
        local_did: String,
        kid: String,
        allowed_issuers: Vec<String>,
        resolver: Arc<dyn DidResolver>,
        client: Client,
    ) -> Self {
        Self {
            key_pair,
            local_did,
            kid,
            allowed_issuers,
            resolver,
            replay: ReplayCache::new(),
            client,
            local_did_document: None,
            holder_state: None,
            issuer_state: None,
            sts_state: None,
            credential_service_path: "/api/credentials/v1".to_string(),
            issuance_service_path: "/api/issuance/v1".to_string(),
        }
    }

    pub(crate) fn new_wallet(config: NativeWalletConfig) -> Self {
        let holder_state = HolderState::new(
            config.key_pair.clone(),
            config.local_did.clone(),
            config.kid.clone(),
            config.client.clone(),
            config.resolver.clone(),
            config.store.clone(),
        );

        let issuer_state = IssuerState::new(
            config.key_pair.clone(),
            config.local_did.clone(),
            config.kid.clone(),
            config.client.clone(),
            config.resolver.clone(),
        );

        let sts_state = StsState::new(
            config.key_pair.clone(),
            config.local_did.clone(),
            config.kid.clone(),
            config.sts_client_id,
            config.sts_client_secret,
        );

        let base = config.base_address.trim_end_matches('/');
        let credential_endpoint = format!("{base}{}", config.credential_service_path);
        let issuer_endpoint = format!("{base}{}", config.issuance_service_path);
        let public_jwk = serde_json::to_value(config.key_pair.public_jwk()).unwrap_or(Value::Null);
        let local_did_document = build_local_did_document(
            &config.local_did,
            &config.kid,
            public_jwk,
            &credential_endpoint,
            &issuer_endpoint,
        );

        Self {
            key_pair: config.key_pair,
            local_did: config.local_did,
            kid: config.kid,
            allowed_issuers: config.allowed_issuers,
            resolver: config.resolver,
            replay: ReplayCache::new(),
            client: config.client,
            local_did_document: Some(local_did_document),
            holder_state: Some(holder_state),
            issuer_state: Some(issuer_state),
            sts_state: Some(sts_state),
            credential_service_path: config.credential_service_path,
            issuance_service_path: config.issuance_service_path,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn credential_store(&self) -> Option<Arc<dyn CredentialStore>> {
        self.holder_state.as_ref().map(|holder| holder.store())
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

#[async_trait]
impl AuthBackend for NativeBackend {
    async fn get_token(
        &self,
        client: &Client,
        remote_address: &str,
        did_web: String,
    ) -> anyhow::Result<String> {
        let target_did = if did_web.is_empty() || did_web == self.local_did {
            crate::connector::utils::derive_did_web(remote_address)?
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
        let url = format!("{trimmed_remote}/auth/token");

        let response = client
            .post(&url)
            .bearer_auth(si_token)
            .send()
            .await?
            .error_for_status()?;

        let token_resp: TokenResponse = response.json().await?;
        Ok(token_resp.access_token)
    }

    async fn did_document(&self, _client: &Client, did: &str) -> anyhow::Result<Value> {
        if did == self.local_did
            && let Some(document) = &self.local_did_document
        {
            return Ok(document.clone());
        }

        let doc = self.resolver.resolve(did).await?;
        Ok(serde_json::to_value(doc)?)
    }

    fn router(&self) -> Router<AppStateAuthentication> {
        Router::new().route("/token", post(auth_token))
    }

    fn extra_routes(&self) -> Option<Router> {
        let holder = self.holder_state.clone()?;
        let issuer = self.issuer_state.clone()?;
        let sts = self.sts_state.clone()?;

        Some(
            Router::new()
                .nest(&self.credential_service_path, holder::router(holder))
                .nest(&self.issuance_service_path, issuer::router(issuer))
                .merge(sts::router(sts)),
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn native(authenticator: &Authenticator) -> Result<&NativeBackend, StatusCode> {
    authenticator
        .backend()
        .as_any()
        .downcast_ref::<NativeBackend>()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)
}

async fn auth_token(
    State(state): State<AppStateAuthentication>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let backend = native(&state.authenticator)?;

    let token_str = bearer_token(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let claims = validate_si_token(
        token_str,
        &backend.local_did,
        backend.resolver.as_ref(),
        &backend.replay,
    )
    .await
    .map_err(|err| {
        tracing::warn!("SI token validation failed: {err}");
        StatusCode::UNAUTHORIZED
    })?;

    let peer_did = claims.sub;
    let forward_token = claims.token;

    let peer_doc = backend
        .resolver
        .resolve(&peer_did)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let endpoint = peer_doc
        .service_endpoint("CredentialService")
        .ok_or(StatusCode::BAD_GATEWAY)?;

    let vp_jwt = query_peer_presentation(
        &backend.client,
        &backend.key_pair,
        &backend.local_did,
        &backend.kid,
        &peer_did,
        endpoint,
        forward_token,
    )
    .await
    .map_err(|err| {
        tracing::warn!("Failed to query peer presentation: {err}");
        StatusCode::UNAUTHORIZED
    })?;

    let vc_jwt = validate_vp(&vp_jwt, &peer_did, &backend.local_did, &peer_doc).map_err(|err| {
        tracing::warn!("VP validation failed: {err}");
        StatusCode::UNAUTHORIZED
    })?;

    let (vc_issuer, subject) = validate_vc(
        &vc_jwt,
        &peer_did,
        &backend.allowed_issuers,
        backend.resolver.as_ref(),
    )
    .await
    .map_err(|err| {
        tracing::warn!("VC validation failed: {err}");
        StatusCode::UNAUTHORIZED
    })?;

    let mut credentials = HashMap::new();
    credentials.insert(
        "identity".to_string(),
        CredentialData {
            r#type: "identity_credential".to_string(),
            format: "vc+jwt".to_string(),
            credential_data: subject,
            issuer: vc_issuer,
        },
    );

    let local_iss = state
        .participant_info
        .did_web()
        .unwrap_or_else(|_| backend.local_did.clone());

    let access_token = state
        .authenticator
        .derive_access_token(credentials, local_iss, peer_did)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": 300
    })))
}

#[cfg(all(test, not(feature = "tck")))]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, header::AUTHORIZATION},
    };
    use chrono::Utc;
    use http_body_util::BodyExt;
    use jsonwebtoken::dangerous::insecure_decode;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::{
        auth::AuthClaims,
        connector::app_state::ParticipantInfo,
        dcp::{
            holder::{self, HolderState},
            issuer::mint_identity_credential,
            si_token::{SiClaims, build_si_token},
            test_support::{
                HOLDER_DID, ISSUER_DID, SharedResolver, TestServer, kid, make_did_document,
                test_key_pair,
            },
        },
        store::credential_store::{CredentialStore, FileCredentialStore, StoredCredential},
    };

    fn test_auth_state(backend: Arc<NativeBackend>) -> AppStateAuthentication {
        let key_pair = test_key_pair();
        let authenticator = Arc::new(Authenticator::new(backend, key_pair));
        AppStateAuthentication {
            client: Client::builder().no_proxy().build().unwrap(),
            participant_info: ParticipantInfo {
                id: "party-a".to_string(),
                external_address: "http://party-a".to_string(),
            },
            authenticator,
        }
    }

    #[tokio::test]
    async fn test_verifier_auth_token_happy_and_rejections() {
        let resolver = Arc::new(SharedResolver::new());
        let key_pair = test_key_pair();

        // 1. Set up Party B (Holder) on a loopback TestServer with an issued credential
        let temp_dir = tempfile::tempdir().unwrap();
        let holder_store = Arc::new(FileCredentialStore::new(temp_dir.path().to_path_buf()));

        let vc = mint_identity_credential(
            &key_pair,
            ISSUER_DID,
            &kid(ISSUER_DID),
            HOLDER_DID,
            json!({
                "email": "ops@party-b.example",
                "address": { "country": "DE" }
            }),
        )
        .unwrap();

        holder_store
            .store(&StoredCredential {
                id: format!("{ISSUER_DID}#pid-1"),
                credential_type: "identity_credential".to_string(),
                format: "vc+jwt".to_string(),
                issuer: ISSUER_DID.to_string(),
                payload: vc,
            })
            .await
            .unwrap();

        let holder_state = HolderState::new(
            key_pair.clone(),
            HOLDER_DID.to_string(),
            kid(HOLDER_DID),
            Client::builder().no_proxy().build().unwrap(),
            resolver.clone(),
            holder_store,
        );
        let holder_app = Router::new().nest("/api/credentials/v1", holder::router(holder_state));
        let holder_server = TestServer::spawn(holder_app).await;
        let holder_endpoint = holder_server.url("/api/credentials/v1");

        // 2. Register DIDs
        resolver.register(
            ISSUER_DID,
            make_did_document(ISSUER_DID, None, Some("http://127.0.0.1:0/api/issuance/v1")),
        );
        resolver.register(
            HOLDER_DID,
            make_did_document(HOLDER_DID, Some(&holder_endpoint), None),
        );

        // 3. Set up Party A (Verifier) with NativeBackend
        let backend = Arc::new(NativeBackend::new(
            key_pair.clone(),
            ISSUER_DID.to_string(),
            kid(ISSUER_DID),
            vec![ISSUER_DID.to_string()],
            resolver.clone(),
            Client::builder().no_proxy().build().unwrap(),
        ));
        let auth_state = test_auth_state(backend.clone());
        let router = backend.router().with_state(auth_state.clone());

        // --- Happy Path ---
        let token = build_si_token(
            &key_pair,
            HOLDER_DID.to_string(),
            ISSUER_DID.to_string(),
            kid(HOLDER_DID),
            None,
        )
        .unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/token")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["token_type"], "Bearer");
        assert_eq!(body["expires_in"], 300);

        let access_token = body["access_token"].as_str().unwrap();
        let decoded = insecure_decode::<AuthClaims>(access_token).unwrap();
        assert_eq!(decoded.claims.issuer().unwrap(), ISSUER_DID);
        assert_eq!(decoded.claims.subject().unwrap(), HOLDER_DID);
        assert_eq!(
            decoded.claims.data.get("email").unwrap(),
            "ops@party-b.example"
        );
        assert_eq!(decoded.claims.data.get("country").unwrap(), "DE");

        // --- Rejection 1: Bad Audience ---
        let bad_aud_token = build_si_token(
            &key_pair,
            HOLDER_DID.to_string(),
            "did:web:wrong-aud".to_string(),
            kid(HOLDER_DID),
            None,
        )
        .unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/token")
            .header(AUTHORIZATION, format!("Bearer {bad_aud_token}"))
            .body(Body::empty())
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // --- Rejection 2: Expired Token ---
        let now = Utc::now().timestamp();
        let expired_claims = SiClaims {
            iss: HOLDER_DID.to_string(),
            sub: HOLDER_DID.to_string(),
            aud: ISSUER_DID.to_string(),
            iat: now - 600,
            nbf: Some(now - 600),
            exp: now - 300,
            jti: uuid::Uuid::new_v4().to_string(),
            token: None,
        };
        let expired_token = key_pair
            .encode_with_kid(&expired_claims, kid(HOLDER_DID))
            .unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/token")
            .header(AUTHORIZATION, format!("Bearer {expired_token}"))
            .body(Body::empty())
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // --- Rejection 3: Replayed jti ---
        // Re-sending the original valid token should fail because its jti is already in replay cache
        let req = Request::builder()
            .method("POST")
            .uri("/token")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // --- Rejection 4: VC Issuer Not in allowed_issuers ---
        let strict_backend = Arc::new(NativeBackend::new(
            key_pair.clone(),
            ISSUER_DID.to_string(),
            kid(ISSUER_DID),
            vec!["did:web:trusted-authority-only".to_string()],
            resolver.clone(),
            Client::builder().no_proxy().build().unwrap(),
        ));
        let strict_auth_state = test_auth_state(strict_backend.clone());
        let strict_router = strict_backend.router().with_state(strict_auth_state);

        let fresh_token = build_si_token(
            &key_pair,
            HOLDER_DID.to_string(),
            ISSUER_DID.to_string(),
            kid(HOLDER_DID),
            None,
        )
        .unwrap();

        let req = Request::builder()
            .method("POST")
            .uri("/token")
            .header(AUTHORIZATION, format!("Bearer {fresh_token}"))
            .body(Body::empty())
            .unwrap();

        let resp = strict_router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_native_backend_get_token() {
        let resolver = Arc::new(SharedResolver::new());
        let key_pair = test_key_pair();
        let http_client = Client::builder().no_proxy().build().unwrap();

        // 1. Party A: Verifier running NativeBackend router at /auth/token
        let backend_a = Arc::new(NativeBackend::new(
            key_pair.clone(),
            ISSUER_DID.to_string(),
            kid(ISSUER_DID),
            vec![ISSUER_DID.to_string()],
            resolver.clone(),
            http_client.clone(),
        ));
        let auth_state_a = test_auth_state(backend_a.clone());
        let party_a_app = Router::new().nest("/auth", backend_a.router().with_state(auth_state_a));
        let party_a_server = TestServer::spawn(party_a_app).await;

        // 2. Party B: Holder running CredentialService at /api/credentials/v1 with stored VC
        let temp_dir = tempfile::tempdir().unwrap();
        let holder_store = Arc::new(FileCredentialStore::new(temp_dir.path().to_path_buf()));

        let vc = mint_identity_credential(
            &key_pair,
            ISSUER_DID,
            &kid(ISSUER_DID),
            HOLDER_DID,
            json!({
                "email": "ops@party-b.example",
                "address": { "country": "DE" }
            }),
        )
        .unwrap();

        holder_store
            .store(&StoredCredential {
                id: format!("{ISSUER_DID}#pid-1"),
                credential_type: "identity_credential".to_string(),
                format: "vc+jwt".to_string(),
                issuer: ISSUER_DID.to_string(),
                payload: vc,
            })
            .await
            .unwrap();

        let holder_state = HolderState::new(
            key_pair.clone(),
            HOLDER_DID.to_string(),
            kid(HOLDER_DID),
            http_client.clone(),
            resolver.clone(),
            holder_store,
        );
        let party_b_app = Router::new().nest("/api/credentials/v1", holder::router(holder_state));
        let party_b_server = TestServer::spawn(party_b_app).await;
        let party_b_endpoint = party_b_server.url("/api/credentials/v1");

        // 3. Register DIDs
        resolver.register(
            ISSUER_DID,
            make_did_document(ISSUER_DID, None, Some("http://127.0.0.1:0/api/issuance/v1")),
        );
        resolver.register(
            HOLDER_DID,
            make_did_document(HOLDER_DID, Some(&party_b_endpoint), None),
        );

        // 4. Party B: NativeBackend calling get_token on Party A
        let backend_b = NativeBackend::new(
            key_pair.clone(),
            HOLDER_DID.to_string(),
            kid(HOLDER_DID),
            vec![ISSUER_DID.to_string()],
            resolver.clone(),
            http_client.clone(),
        );

        let access_token = backend_b
            .get_token(
                &http_client,
                &party_a_server.url(""),
                ISSUER_DID.to_string(),
            )
            .await
            .unwrap();

        let decoded = insecure_decode::<AuthClaims>(&access_token).unwrap();
        assert_eq!(decoded.claims.issuer().unwrap(), ISSUER_DID);
        assert_eq!(decoded.claims.subject().unwrap(), HOLDER_DID);
        assert_eq!(
            decoded.claims.data.get("email").unwrap(),
            "ops@party-b.example"
        );
        assert_eq!(decoded.claims.data.get("country").unwrap(), "DE");
    }
}
