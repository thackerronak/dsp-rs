use std::{collections::HashMap, sync::Arc};

use axum::{Json, Router, extract::State, response::IntoResponse};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use crate::{
    AppError,
    auth::{backend::AuthBackend, extractor::AuthClaims, model::CredentialData},
    connector::app_state::{AppStateAuthentication, KeyPair},
};

pub(crate) mod backend;
pub(crate) mod backends;
pub(crate) mod extractor;
pub(crate) mod model;

#[cfg(test)]
mod tests;

pub(crate) struct Authenticator {
    backend: Arc<dyn AuthBackend>,
    key_pair: KeyPair,
}

impl Authenticator {
    pub(crate) fn new(backend: Arc<dyn AuthBackend>, key_pair: KeyPair) -> Self {
        Self { backend, key_pair }
    }

    pub(crate) fn backend(&self) -> &dyn AuthBackend {
        self.backend.as_ref()
    }

    // TODO: provide offer ID in order to match correct credentials on the provider side
    pub(crate) async fn get_token(
        &self,
        #[cfg_attr(feature = "tck", allow(unused))] client: &Client,
        #[cfg_attr(feature = "tck", allow(unused))] remote_address: &str,
        #[cfg_attr(feature = "tck", allow(unused))] did_web: String,
    ) -> anyhow::Result<String> {
        #[cfg(feature = "tck")]
        return Ok("".to_string());

        #[cfg_attr(feature = "tck", allow(unreachable_code))]
        self.backend
            .get_token(client, remote_address, did_web)
            .await
    }

    pub(crate) async fn did_document(&self, client: &Client, did: &str) -> anyhow::Result<Value> {
        self.backend.did_document(client, did).await
    }

    pub(crate) fn router(&self) -> Router<AppStateAuthentication> {
        self.backend.router()
    }

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

pub(crate) async fn did(
    State(state): State<AppStateAuthentication>,
) -> Result<impl IntoResponse, AppError> {
    let did = state.participant_info.did_web()?;
    let did_document = state
        .authenticator
        .did_document(&state.client, &did)
        .await?;
    Ok(Json(did_document))
}
