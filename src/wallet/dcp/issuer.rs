use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::LOCATION},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{SecondsFormat, Utc};
use reqwest::Client;
use serde_json::{Value, json};

use crate::wallet::{
    KeyPair, bearer_token,
    dcp::{
        model::{
            CredentialContainer, CredentialMessage, CredentialObject, CredentialOfferMessage,
            CredentialRequestMessage, CredentialRequestStatusMessage, DCP_CONTEXT, IssuerMetadata,
        },
        si_token::{ReplayCache, build_si_token, validate_si_token},
    },
    did::DidResolver,
};

const IDENTITY_CREDENTIAL_TYPE: &str = "identity_credential";

/// Format of the credentials this issuer mints, as it goes on the wire. DCP names the
/// VC 1.1-in-JWT envelope `VC1_0_JWT`; holders that parse the field — the Java EDC
/// IdentityHub among them — reject anything outside that vocabulary.
const CREDENTIAL_FORMAT: &str = "VC1_0_JWT";
const CREDENTIAL_VALIDITY_SECS: i64 = 60 * 60 * 24 * 365;

#[derive(Clone)]
pub(crate) struct IssuerState {
    inner: Arc<IssuerInner>,
}

struct IssuerInner {
    key_pair: KeyPair,
    issuer_did: String,
    kid: String,
    client: Client,
    resolver: Arc<dyn DidResolver>,
    replay: ReplayCache,
    sessions: Mutex<HashMap<String, CredentialRequestStatusMessage>>,
}

impl IssuerState {
    pub(crate) fn new(
        key_pair: KeyPair,
        issuer_did: String,
        kid: String,
        client: Client,
        resolver: Arc<dyn DidResolver>,
    ) -> Self {
        Self {
            inner: Arc::new(IssuerInner {
                key_pair,
                issuer_did,
                kid,
                client,
                resolver,
                replay: ReplayCache::new(),
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    async fn deliver(&self, holder_did: String, message: CredentialMessage) -> anyhow::Result<()> {
        let document = self.inner.resolver.resolve(&holder_did).await?;
        let endpoint = document
            .service_endpoint("CredentialService")
            .ok_or_else(|| anyhow::anyhow!("holder exposes no CredentialService"))?
            .to_string();

        let si_token = build_si_token(
            &self.inner.key_pair,
            self.inner.issuer_did.clone(),
            holder_did,
            self.inner.kid.clone(),
            None,
        )?;

        self.inner
            .client
            .post(format!("{endpoint}/credentials"))
            .bearer_auth(si_token)
            .json(&message)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}

pub(crate) fn router(state: IssuerState) -> Router {
    Router::new()
        .route("/metadata", get(metadata))
        .route("/credentials", post(issue_credential))
        // DCP places the status resource directly under the Issuer Service endpoint; the
        // nested path is kept for clients that follow the Location header we used to send.
        .route("/requests/{issuer_pid}", get(request_status))
        .route("/credentials/requests/{issuer_pid}", get(request_status))
        .route("/offer", post(trigger_offer))
        .with_state(state)
}

pub(crate) fn mint_identity_credential(
    key_pair: &KeyPair,
    issuer_did: &str,
    kid: &str,
    holder_did: &str,
    subject: Value,
) -> anyhow::Result<String> {
    let issued_at = Utc::now();
    let expires_at = issued_at + chrono::Duration::seconds(CREDENTIAL_VALIDITY_SECS);
    let timestamp = |t: chrono::DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, true);

    // Name the holder in the subject: a verifier binds the presentation to this.
    let mut subject = subject;
    if let Some(map) = subject.as_object_mut() {
        map.insert("id".to_string(), Value::String(holder_did.to_string()));
    }

    let claims = json!({
        "iss": issuer_did,
        "sub": holder_did,
        "iat": issued_at.timestamp(),
        "exp": expires_at.timestamp(),
        "jti": uuid::Uuid::new_v4().to_string(),
        "vc": {
            "@context": ["https://www.w3.org/2018/credentials/v1"],
            "id": format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            // A holder answers a presentation query by matching this against the credential
            // type in the scope, so it has to be the same string the scope grammar uses.
            "type": ["VerifiableCredential", IDENTITY_CREDENTIAL_TYPE],
            "issuer": issuer_did,
            // Required by the VC 1.1 data model. A credential without it is rejected
            // outright by the Java EDC.
            "issuanceDate": timestamp(issued_at),
            "expirationDate": timestamp(expires_at),
            "credentialSubject": subject
        }
    });

    key_pair.encode_with_kid(claims, kid.to_string())
}

fn default_subject(holder_did: &str) -> Value {
    let host = holder_did
        .strip_prefix("did:web:")
        .unwrap_or(holder_did)
        .split(':')
        .next()
        .unwrap_or("holder")
        .replace("%3A", "-");

    json!({
        "email": format!("ops@{host}.example"),
        "given_name": "Auto",
        "family_name": "Parts",
        "address": { "country": "DE", "locality": "Potsdam", "region": "Brandenburg" }
    })
}

fn identity_credential_object() -> CredentialObject {
    CredentialObject {
        id: IDENTITY_CREDENTIAL_TYPE.to_string(),
        credential_type: IDENTITY_CREDENTIAL_TYPE.to_string(),
        profile: None,
        binding_methods: Vec::new(),
        schema: None,
    }
}

async fn authenticate(state: &IssuerState, headers: &HeaderMap) -> Result<String, StatusCode> {
    let token = bearer_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;

    let claims = validate_si_token(
        token,
        &state.inner.issuer_did,
        state.inner.resolver.as_ref(),
        &state.inner.replay,
    )
    .await
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    Ok(claims.sub)
}

async fn metadata(State(state): State<IssuerState>) -> impl IntoResponse {
    Json(IssuerMetadata {
        issuer: state.inner.issuer_did.clone(),
        credentials_supported: vec![identity_credential_object()],
    })
}

async fn issue_credential(
    State(state): State<IssuerState>,
    headers: HeaderMap,
    Json(request): Json<CredentialRequestMessage>,
) -> Result<impl IntoResponse, StatusCode> {
    let holder_did = authenticate(&state, &headers).await?;

    let payload = mint_identity_credential(
        &state.inner.key_pair,
        &state.inner.issuer_did,
        &state.inner.kid,
        &holder_did,
        default_subject(&holder_did),
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let issuer_pid = uuid::Uuid::new_v4().to_string();

    let status = CredentialRequestStatusMessage {
        context: vec![DCP_CONTEXT.to_string()],
        message_type: "CredentialRequestStatusMessage".to_string(),
        issuer_pid: issuer_pid.clone(),
        holder_pid: request.holder_pid.clone(),
        status: "ISSUED".to_string(),
    };

    state
        .inner
        .sessions
        .lock()
        .expect("issuer sessions mutex poisoned")
        .insert(issuer_pid.clone(), status);

    let message = CredentialMessage {
        context: vec![DCP_CONTEXT.to_string()],
        message_type: "CredentialMessage".to_string(),
        issuer_pid: issuer_pid.clone(),
        holder_pid: request.holder_pid,
        status: "ISSUED".to_string(),
        credentials: vec![CredentialContainer {
            credential_type: IDENTITY_CREDENTIAL_TYPE.to_string(),
            payload,
            format: CREDENTIAL_FORMAT.to_string(),
            r#type: "CredentialContainer".to_string(),
        }],
    };

    let delivery_state = state.clone();
    let delivery_holder = holder_did;
    tokio::spawn(async move {
        if let Err(err) = delivery_state.deliver(delivery_holder, message).await {
            tracing::warn!("credential delivery failed: {err}");
        }
    });

    let location = format!("/api/issuance/v1/requests/{issuer_pid}");
    Ok((StatusCode::CREATED, [(LOCATION, location)]))
}

async fn request_status(
    State(state): State<IssuerState>,
    Path(issuer_pid): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let status = state
        .inner
        .sessions
        .lock()
        .expect("issuer sessions mutex poisoned")
        .get(&issuer_pid)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(status))
}

async fn trigger_offer(
    State(state): State<IssuerState>,
    Json(holder_did): Json<Value>,
) -> Result<impl IntoResponse, StatusCode> {
    let holder_did = holder_did
        .get("holderDid")
        .and_then(Value::as_str)
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();

    let document = state
        .inner
        .resolver
        .resolve(&holder_did)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let endpoint = document
        .service_endpoint("CredentialService")
        .ok_or(StatusCode::BAD_GATEWAY)?
        .to_string();

    let si_token = build_si_token(
        &state.inner.key_pair,
        state.inner.issuer_did.clone(),
        holder_did,
        state.inner.kid.clone(),
        None,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let offer = CredentialOfferMessage {
        context: vec![DCP_CONTEXT.to_string()],
        message_type: "CredentialOfferMessage".to_string(),
        issuer: state.inner.issuer_did.clone(),
        credentials: vec![identity_credential_object()],
    };

    state
        .inner
        .client
        .post(format!("{endpoint}/offers"))
        .bearer_auth(si_token)
        .json(&offer)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::test_support::{
        HOLDER_DID, ISSUER_DID, holder_si_token, kid, static_resolver, test_key_pair,
    };
    use axum::{
        body::Body,
        http::{Request, header::AUTHORIZATION},
    };
    use http_body_util::BodyExt;
    use jsonwebtoken::dangerous::insecure_decode;
    use tower::ServiceExt;

    fn issuer_state() -> IssuerState {
        IssuerState::new(
            test_key_pair(),
            ISSUER_DID.to_string(),
            kid(ISSUER_DID),
            Client::new(),
            Arc::new(static_resolver()),
        )
    }

    #[test]
    fn test_issuer_routes_mint_identity_credential_format() {
        let payload = mint_identity_credential(
            &test_key_pair(),
            ISSUER_DID,
            &kid(ISSUER_DID),
            HOLDER_DID,
            default_subject(HOLDER_DID),
        )
        .unwrap();

        let claims = insecure_decode::<Value>(&payload).unwrap().claims;
        assert_eq!(claims["iss"], ISSUER_DID);
        assert_eq!(claims["sub"], HOLDER_DID);
        assert_eq!(claims["vc"]["type"][1], IDENTITY_CREDENTIAL_TYPE);
        assert!(claims["vc"]["issuanceDate"].is_string());
        assert!(claims["vc"]["expirationDate"].is_string());
        assert!(claims["vc"]["credentialSubject"]["email"].is_string());
        assert_eq!(
            claims["vc"]["credentialSubject"]["address"]["country"],
            "DE"
        );
    }

    #[tokio::test]
    async fn test_issuer_routes_issue_requires_valid_token() {
        let request = Request::builder()
            .method("POST")
            .uri("/credentials")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "CredentialRequestMessage",
                    "holderPid": "holder-pid-1",
                    "credentials": [{ "id": "identity_credential" }]
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(issuer_state()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_issuer_routes_issue_credential_returns_created() {
        let token = holder_si_token(ISSUER_DID);

        let request = Request::builder()
            .method("POST")
            .uri("/credentials")
            .header("content-type", "application/json")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "@context": [DCP_CONTEXT],
                    "type": "CredentialRequestMessage",
                    "holderPid": "holder-pid-1",
                    "credentials": [{ "id": "identity_credential" }]
                }))
                .unwrap(),
            ))
            .unwrap();

        let response = router(issuer_state()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        assert!(response.headers().get(LOCATION).is_some());
    }

    #[tokio::test]
    async fn test_issuer_routes_metadata() {
        let request = Request::builder()
            .method("GET")
            .uri("/metadata")
            .body(Body::empty())
            .unwrap();

        let response = router(issuer_state()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let metadata: IssuerMetadata = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(metadata.issuer, ISSUER_DID);
        assert_eq!(metadata.credentials_supported.len(), 1);
    }
}
