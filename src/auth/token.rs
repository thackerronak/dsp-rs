//! `POST /auth/token` — the verifier side of the DCP token exchange.
//!
//! A peer presents a Self-Issued ID Token; we validate it, pull a Verifiable Presentation
//! from the peer's Credential Service, validate the VP and the VC inside it, and mint a
//! DSP access token. One synchronous round trip, no session and no polling.

use std::collections::HashMap;

use axum::{Json, extract::State, http::HeaderMap, http::StatusCode, response::IntoResponse};
use serde_json::{Value, json};

use crate::{
    auth::model::CredentialData,
    connector::app_state::AppStateAuthentication,
    wallet::{
        bearer_token,
        dcp::{
            si_token::validate_si_token,
            verifier::{query_peer_presentation, validate_vc, validate_vp},
        },
    },
};

pub(super) async fn auth_token(
    State(state): State<AppStateAuthentication>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let wallet = &state.authenticator;

    let token_str = bearer_token(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let claims = validate_si_token(
        token_str,
        &wallet.local_did,
        wallet.resolver.as_ref(),
        &wallet.replay,
    )
    .await
    .map_err(|err| {
        tracing::warn!("SI token validation failed: {err}");
        StatusCode::UNAUTHORIZED
    })?;

    let peer_did = claims.sub;
    let forward_token = claims.token;

    let peer_doc = wallet
        .resolver
        .resolve(&peer_did)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let endpoint = peer_doc
        .service_endpoint("CredentialService")
        .ok_or(StatusCode::BAD_GATEWAY)?;

    let vp_jwt = query_peer_presentation(
        &wallet.client,
        &wallet.key_pair,
        &wallet.local_did,
        &wallet.kid,
        &peer_did,
        endpoint,
        forward_token,
    )
    .await
    .map_err(|err| {
        tracing::warn!("Failed to query peer presentation: {err}");
        StatusCode::UNAUTHORIZED
    })?;

    let vc_jwt = validate_vp(&vp_jwt, &peer_did, &wallet.local_did, &peer_doc).map_err(|err| {
        tracing::warn!("VP validation failed: {err}");
        StatusCode::UNAUTHORIZED
    })?;

    let (vc_issuer, subject) = validate_vc(
        &vc_jwt,
        &peer_did,
        &wallet.allowed_issuers,
        wallet.resolver.as_ref(),
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
        .unwrap_or_else(|_| wallet.local_did.clone());

    let access_token = state
        .authenticator
        .derive_access_token(credentials, local_iss, peer_did)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": 300
    })) as Json<Value>)
}
