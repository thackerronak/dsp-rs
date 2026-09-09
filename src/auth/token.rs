//! `POST /auth/token` — mints a DSP access token from a peer's Self-Issued ID Token.
//!
//! Kept for peers that ask for an access token before making protocol requests. The DCP
//! path needs no pre-flight: the Self-Issued ID Token travels on the DSP request itself
//! and is verified inline (`Authenticator::authenticate`). Both share one verifier.

use axum::{Json, extract::State, http::HeaderMap, http::StatusCode, response::IntoResponse};
use serde_json::{Value, json};

use crate::{connector::app_state::AppStateAuthentication, wallet::bearer_token};

pub(super) async fn auth_token(
    State(state): State<AppStateAuthentication>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let wallet = &state.authenticator;

    let token_str = bearer_token(&headers).ok_or(StatusCode::UNAUTHORIZED)?;

    let (peer_did, credentials) =
        wallet
            .verify_peer_credentials(token_str)
            .await
            .map_err(|err| {
                tracing::warn!("Peer credential verification failed: {err}");
                StatusCode::UNAUTHORIZED
            })?;

    let local_iss = state
        .participant_info
        .did_web()
        .unwrap_or_else(|_| wallet.local_did.clone());

    let access_token = wallet
        .derive_access_token(credentials, local_iss, peer_did)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": 300
    })) as Json<Value>)
}
