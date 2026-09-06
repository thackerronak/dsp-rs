use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Form, State},
    http::StatusCode,
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{connector::app_state::KeyPair, dcp::si_token::build_si_token};

const TOKEN_TTL_SECS: u64 = 300;

#[derive(Clone)]
pub(crate) struct StsState {
    inner: Arc<StsInner>,
}

struct StsInner {
    key_pair: KeyPair,
    issuer_did: String,
    kid: String,
    client_id: String,
    client_secret: String,
}

impl StsState {
    pub(crate) fn new(
        key_pair: KeyPair,
        issuer_did: String,
        kid: String,
        client_id: String,
        client_secret: String,
    ) -> Self {
        Self {
            inner: Arc::new(StsInner {
                key_pair,
                issuer_did,
                kid,
                client_id,
                client_secret,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: String,
    client_id: String,
    client_secret: String,
    audience: String,
    token: Option<String>,
    bearer_access_scope: Option<String>,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

pub(crate) fn router(state: StsState) -> Router {
    Router::new().route("/token", post(token)).with_state(state)
}

fn error(status: StatusCode, error: &str, description: &str) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!([{ "error": error, "error_description": description }])),
    )
}

async fn token(
    State(state): State<StsState>,
    Form(request): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, (StatusCode, Json<Value>)> {
    let inner = &state.inner;

    if request.grant_type != "client_credentials" {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be client_credentials",
        ));
    }

    if request.client_id != inner.client_id || request.client_secret != inner.client_secret {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client_id or client_secret is invalid",
        ));
    }

    let token_claim = request.bearer_access_scope.and(request.token);

    let access_token = build_si_token(
        &inner.key_pair,
        inner.issuer_did.clone(),
        request.audience,
        inner.kid.clone(),
        token_claim,
    )
    .map_err(|err| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &err.to_string(),
        )
    })?;

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: TOKEN_TTL_SECS,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcp::si_token::SiClaims;
    use axum::{
        body::Body,
        http::{Request, header::CONTENT_TYPE},
    };
    use http_body_util::BodyExt;
    use jsonwebtoken::dangerous::insecure_decode;
    use tower::ServiceExt;

    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

    const ISSUER_DID: &str = "did:web:party-a";

    fn test_router() -> Router {
        let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).unwrap();
        let state = StsState::new(
            key_pair,
            ISSUER_DID.to_string(),
            format!("{ISSUER_DID}#keys-1"),
            "dsp-client".to_string(),
            "dsp-secret".to_string(),
        );
        router(state)
    }

    async fn post_form(router: Router, body: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method("POST")
            .uri("/token")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap();
        (status, value)
    }

    #[tokio::test]
    async fn test_sts_token_endpoint() {
        let body = "grant_type=client_credentials&client_id=dsp-client\
            &client_secret=dsp-secret&audience=did:web:party-b";

        let (status, value) = post_form(test_router(), body).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["token_type"], "Bearer");
        assert_eq!(value["expires_in"], 300);

        let access_token = value["access_token"].as_str().unwrap();
        let claims = insecure_decode::<SiClaims>(access_token).unwrap().claims;
        assert_eq!(claims.iss, ISSUER_DID);
        assert_eq!(claims.sub, ISSUER_DID);
        assert_eq!(claims.aud, "did:web:party-b");
        assert!(claims.token.is_none());
    }

    #[tokio::test]
    async fn test_sts_token_endpoint_embeds_scoped_token() {
        let body = "grant_type=client_credentials&client_id=dsp-client\
            &client_secret=dsp-secret&audience=did:web:party-b\
            &token=vp-access&bearer_access_scope=org.eclipse.dspace.dcp.vc.type:identity_credential";

        let (status, value) = post_form(test_router(), body).await;

        assert_eq!(status, StatusCode::OK);
        let access_token = value["access_token"].as_str().unwrap();
        let claims = insecure_decode::<SiClaims>(access_token).unwrap().claims;
        assert_eq!(claims.token.as_deref(), Some("vp-access"));
    }

    #[tokio::test]
    async fn test_sts_token_endpoint_rejects_bad_grant() {
        let body = "grant_type=authorization_code&client_id=dsp-client\
            &client_secret=dsp-secret&audience=did:web:party-b";

        let (status, _) = post_form(test_router(), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_sts_token_endpoint_rejects_bad_client_secret() {
        let body = "grant_type=client_credentials&client_id=dsp-client\
            &client_secret=wrong&audience=did:web:party-b";

        let (status, _) = post_form(test_router(), body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
