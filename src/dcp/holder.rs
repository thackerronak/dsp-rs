use std::{collections::HashSet, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use reqwest::Client;
use serde_json::Value;

use crate::{
    shared::KeyPair,
    dcp::{
        bearer_token,
        model::{
            CredentialMessage, CredentialOfferMessage, CredentialReference,
            CredentialRequestMessage, DCP_CONTEXT, PresentationQueryMessage,
            PresentationResponseMessage,
        },
        scope::{ScopeQuery, mint_jwt_vp},
        si_token::{DidResolver, ReplayCache, build_si_token, validate_si_token},
    },
    dcp::store::{CredentialStore, StoredCredential},
};

#[derive(Clone)]
pub(crate) struct HolderState {
    inner: Arc<HolderInner>,
}

struct HolderInner {
    key_pair: KeyPair,
    holder_did: String,
    kid: String,
    client: Client,
    resolver: Arc<dyn DidResolver>,
    store: Arc<dyn CredentialStore>,
    replay: ReplayCache,
}

impl HolderState {
    pub(crate) fn new(
        key_pair: KeyPair,
        holder_did: String,
        kid: String,
        client: Client,
        resolver: Arc<dyn DidResolver>,
        store: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            inner: Arc::new(HolderInner {
                key_pair,
                holder_did,
                kid,
                client,
                resolver,
                store,
                replay: ReplayCache::new(),
            }),
        }
    }

    pub(crate) fn store(&self) -> Arc<dyn CredentialStore> {
        self.inner.store.clone()
    }
}

/// The holder's Credential Service.
///
/// `dcp_issuance` gates the routes that only matter when credentials are obtained
/// over DCP: receiving a pushed credential, receiving an offer, and asking an issuer
/// for one. With an external OID4VCI issuer those are unused, and leaving them
/// mounted is surface for a feature that is not in play. Listing credentials and
/// answering presentation queries are always available — they are how the wallet
/// does its job regardless of where the credential came from.
pub(crate) fn router(state: HolderState, dcp_issuance: bool) -> Router {
    let credentials = if dcp_issuance {
        get(list_credentials).post(receive_credential)
    } else {
        get(list_credentials)
    };

    let mut router = Router::new()
        .route("/credentials", credentials)
        .route("/presentations/query", post(query_presentations));

    if dcp_issuance {
        router = router
            .route("/offers", post(receive_offer))
            .route("/request", post(trigger_request));
    }

    router.with_state(state)
}

async fn authenticate(state: &HolderState, headers: &HeaderMap) -> Result<String, StatusCode> {
    let token = bearer_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;

    let claims = validate_si_token(
        token,
        &state.inner.holder_did,
        state.inner.resolver.as_ref(),
        &state.inner.replay,
    )
    .await
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    Ok(claims.sub)
}

async fn receive_credential(
    State(state): State<HolderState>,
    headers: HeaderMap,
    Json(message): Json<CredentialMessage>,
) -> Result<impl IntoResponse, StatusCode> {
    let issuer_did = authenticate(&state, &headers).await?;

    for container in &message.credentials {
        let stored = StoredCredential {
            id: format!("{}#{}", issuer_did, message.issuer_pid),
            credential_type: container.credential_type.clone(),
            format: container.format.clone(),
            issuer: issuer_did.clone(),
            payload: container.payload.clone(),
        };

        state
            .inner
            .store
            .store(&stored)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(StatusCode::OK)
}

async fn list_credentials(
    State(state): State<HolderState>,
) -> Result<impl IntoResponse, StatusCode> {
    let credentials = state
        .inner
        .store
        .list()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(credentials))
}

async fn receive_offer(
    State(state): State<HolderState>,
    headers: HeaderMap,
    Json(offer): Json<CredentialOfferMessage>,
) -> Result<impl IntoResponse, StatusCode> {
    let _ = authenticate(&state, &headers).await?;

    request_from_issuer(&state, &offer.issuer)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(StatusCode::ACCEPTED)
}

async fn trigger_request(
    State(state): State<HolderState>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, StatusCode> {
    let issuer_did = body
        .get("issuerDid")
        .and_then(Value::as_str)
        .ok_or(StatusCode::BAD_REQUEST)?;

    request_from_issuer(&state, issuer_did)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(StatusCode::ACCEPTED)
}

async fn request_from_issuer(state: &HolderState, issuer_did: &str) -> anyhow::Result<()> {
    let document = state.inner.resolver.resolve(issuer_did).await?;
    let endpoint = document
        .service_endpoint("IssuerService")
        .ok_or_else(|| anyhow::anyhow!("issuer exposes no IssuerService"))?
        .to_string();

    let si_token = build_si_token(
        &state.inner.key_pair,
        state.inner.holder_did.clone(),
        issuer_did.to_string(),
        state.inner.kid.clone(),
        None,
    )?;

    let request = CredentialRequestMessage {
        context: vec![DCP_CONTEXT.to_string()],
        message_type: "CredentialRequestMessage".to_string(),
        holder_pid: uuid::Uuid::new_v4().to_string(),
        credentials: vec![CredentialReference {
            id: "identity_credential".to_string(),
        }],
    };

    state
        .inner
        .client
        .post(format!("{endpoint}/credentials"))
        .bearer_auth(si_token)
        .json(&request)
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

async fn query_presentations(
    State(state): State<HolderState>,
    headers: HeaderMap,
    Json(query): Json<PresentationQueryMessage>,
) -> Result<impl IntoResponse, StatusCode> {
    let verifier_did = authenticate(&state, &headers).await?;

    if query.presentation_definition.is_some() {
        return Err(StatusCode::NOT_IMPLEMENTED);
    }

    let scopes = match query.scope {
        Some(scopes) if !scopes.is_empty() => scopes,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let all_credentials = state
        .inner
        .store
        .list()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut matched_ids = HashSet::new();
    let mut matched_vps = Vec::new();

    for scope_str in scopes {
        if let Some(scope) = ScopeQuery::parse(&scope_str) {
            for cred in &all_credentials {
                let is_match = match &scope {
                    ScopeQuery::Type(ty) => cred.credential_type == *ty,
                    ScopeQuery::Id(id) => cred.id == *id,
                };

                if is_match && matched_ids.insert(cred.id.clone()) {
                    let vp = mint_jwt_vp(
                        &state.inner.key_pair,
                        &state.inner.holder_did,
                        &state.inner.kid,
                        &verifier_did,
                        &cred.payload,
                    )
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                    matched_vps.push(Value::String(vp));
                }
            }
        }
    }

    Ok(Json(PresentationResponseMessage {
        context: vec![DCP_CONTEXT.to_string()],
        message_type: "PresentationResponseMessage".to_string(),
        presentation: matched_vps,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dcp::test_support::{
            HOLDER_DID, ISSUER_DID, issuer_si_token, kid, static_resolver, test_key_pair,
        },
        dcp::store::FileCredentialStore,
    };
    use axum::{
        body::Body,
        http::{Request, header::AUTHORIZATION},
    };
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    fn holder_state(dir: &std::path::Path) -> HolderState {
        HolderState::new(
            test_key_pair(),
            HOLDER_DID.to_string(),
            kid(HOLDER_DID),
            Client::new(),
            Arc::new(static_resolver()),
            Arc::new(FileCredentialStore::new(dir.to_path_buf())),
        )
    }

    #[tokio::test]
    async fn test_dcp_issuance_routes_are_gated_off() {
        let dir = tempfile::tempdir().unwrap();
        let state = holder_state(dir.path());

        // The DCP-issuance triggers must not exist when the flag is off.
        for (method, uri) in [("POST", "/request"), ("POST", "/offers")] {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap();

            let response = router(state.clone(), false).oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} {uri} should not be mounted when DCP issuance is off"
            );
        }

        // Neither should an issuer be able to push a credential at us.
        let push = Request::builder()
            .method("POST")
            .uri("/credentials")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let response = router(state.clone(), false).oneshot(push).await.unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

        // But the wallet still does its own job: listing and presenting.
        let list = Request::builder()
            .method("GET")
            .uri("/credentials")
            .body(Body::empty())
            .unwrap();
        let response = router(state.clone(), false).oneshot(list).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Present is reachable (unauthenticated here, so it rejects on the token).
        let query = Request::builder()
            .method("POST")
            .uri("/presentations/query")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let response = router(state, false).oneshot(query).await.unwrap();
        assert_ne!(
            response.status(),
            StatusCode::NOT_FOUND,
            "presentation query must stay mounted"
        );
    }

    #[tokio::test]
    async fn test_holder_routes_receive_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let state = holder_state(dir.path());
        let token = issuer_si_token(HOLDER_DID);

        let deliver = Request::builder()
            .method("POST")
            .uri("/credentials")
            .header("content-type", "application/json")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "CredentialMessage",
                    "issuerPid": "issuer-pid-1",
                    "holderPid": "holder-pid-1",
                    "status": "ISSUED",
                    "credentials": [{
                        "credentialType": "identity_credential",
                        "payload": "eyJhbGciOiJFUzI1NiJ9.body.sig",
                        "format": "vc+jwt",
                        "type": "CredentialContainer"
                    }]
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(state.clone(), true).oneshot(deliver).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let list = Request::builder()
            .method("GET")
            .uri("/credentials")
            .body(Body::empty())
            .unwrap();

        let response = router(state, true).oneshot(list).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let stored: Vec<StoredCredential> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].credential_type, "identity_credential");
        assert_eq!(stored[0].issuer, ISSUER_DID);
    }

    #[tokio::test]
    async fn test_holder_routes_receive_requires_valid_token() {
        let dir = tempfile::tempdir().unwrap();
        let state = holder_state(dir.path());

        let deliver = Request::builder()
            .method("POST")
            .uri("/credentials")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "CredentialMessage",
                    "issuerPid": "issuer-pid-1",
                    "holderPid": "holder-pid-1",
                    "status": "ISSUED",
                    "credentials": []
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(state, true).oneshot(deliver).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_presentation_query() {
        use jsonwebtoken::dangerous::insecure_decode;

        let dir = tempfile::tempdir().unwrap();
        let state = holder_state(dir.path());

        // Pre-seed a stored credential
        let cred = StoredCredential {
            id: format!("{ISSUER_DID}#pid-123"),
            credential_type: "identity_credential".to_string(),
            format: "vc+jwt".to_string(),
            issuer: ISSUER_DID.to_string(),
            payload: "eyJhbGciOiJFUzI1NiJ9.vc.payload".to_string(),
        };
        state.inner.store.store(&cred).await.unwrap();

        // 1. Happy path query by type
        let token = issuer_si_token(HOLDER_DID);
        let request = Request::builder()
            .method("POST")
            .uri("/presentations/query")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "PresentationQueryMessage",
                    "scope": ["org.eclipse.dspace.dcp.vc.type:identity_credential"]
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(state.clone(), true).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let resp_msg: PresentationResponseMessage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp_msg.presentation.len(), 1);

        let vp_str = resp_msg.presentation[0].as_str().unwrap();
        let decoded = insecure_decode::<Value>(vp_str).unwrap();
        let claims = decoded.claims;
        assert_eq!(claims["iss"], HOLDER_DID);
        assert_eq!(claims["sub"], HOLDER_DID);
        assert_eq!(claims["aud"], ISSUER_DID);
        let vp = &claims["vp"];
        assert_eq!(
            vp["verifiableCredential"][0],
            "eyJhbGciOiJFUzI1NiJ9.vc.payload"
        );

        // 2. presentationDefinition present => 501 Not Implemented
        let token = issuer_si_token(HOLDER_DID);
        let request = Request::builder()
            .method("POST")
            .uri("/presentations/query")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "PresentationQueryMessage",
                    "presentationDefinition": { "id": "pd-1" }
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(state.clone(), true).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);

        // 3. Empty scope => 400 Bad Request
        let token = issuer_si_token(HOLDER_DID);
        let request = Request::builder()
            .method("POST")
            .uri("/presentations/query")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "PresentationQueryMessage",
                    "scope": []
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(state.clone(), true).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        // 4. Missing token => 401 Unauthorized
        let request = Request::builder()
            .method("POST")
            .uri("/presentations/query")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "PresentationQueryMessage",
                    "scope": ["org.eclipse.dspace.dcp.vc.type:identity_credential"]
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(state, true).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
