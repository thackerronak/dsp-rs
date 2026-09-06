use std::{collections::HashMap, env, sync::Arc};

use anyhow::Context;
use axum::extract::FromRef;
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode, jwk::Jwk,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    sync::mpsc::{Receiver, Sender, channel},
};

use crate::{
    auth::{
        Authenticator,
        backend::AuthBackend,
        backends::native::{NativeBackend, NativeWalletConfig},
    },
    connector::{utils::derive_did_web, validator::SchemaValidator},
    dcp::resolver::HttpDidResolver,
    negotiation::NegotiationEvent,
    store::{Store, credential_store::FileCredentialStore},
    transfer::TransferEvent,
};

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct RemoteConnector {
    pub(crate) remote_address: String,

    #[serde(default = "default_sync_interval")]
    pub(crate) sync_interval_secs: u64,
}

fn default_sync_interval() -> u64 {
    600
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct ParticipantInfo {
    pub(crate) id: String,
    pub(crate) external_address: String,
}

impl ParticipantInfo {
    pub(crate) fn did_web(&self) -> anyhow::Result<String> {
        derive_did_web(&self.external_address)
    }

    pub(crate) fn callback_address(&self) -> String {
        #[cfg(feature = "tck")]
        return format!("{}{}", self.external_address, super::DSP_API_PATH_2025_1);

        // NOTE: we don't include the path for 2025-1
        #[cfg(not(feature = "tck"))]
        self.external_address.to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Configuration {
    participant_info: ParticipantInfo,

    // Used for signing access tokens. The corresponding public key is published in the
    // connector's DID document.
    private_key_pem: String,

    allowed_issuers: Vec<String>,

    federation: HashMap<String, RemoteConnector>,

    #[serde(default)]
    auth: AuthConfig,
}

#[derive(Debug, Deserialize, Default)]
struct AuthConfig {
    #[serde(default)]
    #[allow(dead_code)]
    backend: AuthBackendKind,

    #[serde(default)]
    native: NativeConfig,
}

#[derive(Debug, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
enum AuthBackendKind {
    #[default]
    Native,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub(crate) struct NativeConfig {
    #[serde(default = "default_credential_store_path")]
    pub(crate) credential_store_path: String,

    #[serde(default = "default_credential_service_path")]
    pub(crate) credential_service_path: String,

    #[serde(default = "default_issuance_service_path")]
    pub(crate) issuance_service_path: String,

    #[serde(default = "default_true")]
    pub(crate) issuer_enabled: bool,

    #[serde(default = "default_sts_client_id")]
    pub(crate) sts_client_id: Option<String>,

    #[serde(default = "default_sts_client_secret")]
    pub(crate) sts_client_secret: Option<String>,
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            credential_store_path: default_credential_store_path(),
            credential_service_path: default_credential_service_path(),
            issuance_service_path: default_issuance_service_path(),
            issuer_enabled: true,
            sts_client_id: default_sts_client_id(),
            sts_client_secret: default_sts_client_secret(),
        }
    }
}

fn default_credential_store_path() -> String {
    "data/credentials".to_string()
}

fn default_credential_service_path() -> String {
    "/api/credentials/v1".to_string()
}

fn default_issuance_service_path() -> String {
    "/api/issuance/v1".to_string()
}

fn default_true() -> bool {
    true
}

fn default_sts_client_id() -> Option<String> {
    Some("dsp-client".to_string())
}

fn default_sts_client_secret() -> Option<String> {
    Some("dsp-secret".to_string())
}

pub(crate) struct AppState<T: Store> {
    participant_info: ParticipantInfo,
    federation: Arc<HashMap<String, RemoteConnector>>,
    pub(crate) validator: Arc<SchemaValidator>,
    pub(crate) authenticator: Arc<Authenticator>,
    client: Client,
    store: Arc<T>,
    negotiation_events: Sender<NegotiationEvent>,
    transfer_events: Sender<TransferEvent>,
}

impl<T: Store> Clone for AppState<T> {
    fn clone(&self) -> Self {
        Self {
            participant_info: self.participant_info.clone(),
            federation: self.federation.clone(),
            validator: self.validator.clone(),
            authenticator: self.authenticator.clone(),
            store: self.store.clone(),
            client: self.client.clone(),
            negotiation_events: self.negotiation_events.clone(),
            transfer_events: self.transfer_events.clone(),
        }
    }
}

impl<T: Store> AppState<T> {
    pub(crate) async fn new(
        store: T,
    ) -> anyhow::Result<(Self, Receiver<NegotiationEvent>, Receiver<TransferEvent>)> {
        let config_path = env::var("CONFIG_PATH").unwrap_or("config.json".into());

        let data = fs::read(config_path)
            .await
            .context("Failed to load configuration file")?;
        let config: Configuration =
            serde_json::from_slice(&data).context("Failed to deserialize config")?;

        Self::from_configuration(config, store).await
    }

    #[cfg(test)]
    pub(crate) async fn from_config_json(
        value: serde_json::Value,
        store: T,
    ) -> anyhow::Result<(Self, Receiver<NegotiationEvent>, Receiver<TransferEvent>)> {
        let config: Configuration =
            serde_json::from_value(value).context("Failed to deserialize config")?;
        Self::from_configuration(config, store).await
    }

    async fn from_configuration(
        config: Configuration,
        store: T,
    ) -> anyhow::Result<(Self, Receiver<NegotiationEvent>, Receiver<TransferEvent>)> {
        let validator = SchemaValidator::new().await?;
        let key_pair = KeyPair::from_ec_pem(&config.private_key_pem)?;
        let client = Client::new();

        let native = config.auth.native;
        let local_did = config.participant_info.did_web()?;
        let kid = format!("{local_did}#keys-1");
        let resolver = Arc::new(HttpDidResolver::new(client.clone()));
        let credential_store = Arc::new(FileCredentialStore::new(
            native.credential_store_path.into(),
        ));

        let backend: Arc<dyn AuthBackend> =
            Arc::new(NativeBackend::new_wallet(NativeWalletConfig {
                key_pair: key_pair.clone(),
                local_did,
                kid,
                allowed_issuers: config.allowed_issuers.clone(),
                resolver,
                client: client.clone(),
                store: credential_store,
                base_address: config.participant_info.external_address.clone(),
                credential_service_path: native.credential_service_path,
                issuance_service_path: native.issuance_service_path,
                sts_client_id: native
                    .sts_client_id
                    .unwrap_or_else(|| "dsp-client".to_string()),
                sts_client_secret: native
                    .sts_client_secret
                    .unwrap_or_else(|| "dsp-secret".to_string()),
            }));

        let (tx_n, rx_n) = channel::<NegotiationEvent>(10);
        let (tx_t, rx_t) = channel::<TransferEvent>(10);

        Ok((
            Self {
                participant_info: config.participant_info,
                federation: Arc::new(config.federation),
                validator: Arc::new(validator),
                authenticator: Arc::new(Authenticator::new(backend, key_pair)),
                store: Arc::new(store),
                client,
                negotiation_events: tx_n,
                transfer_events: tx_t,
            },
            rx_n,
            rx_t,
        ))
    }
}

pub(crate) struct AppStateAPI<T: Store> {
    pub(crate) store: Arc<T>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateAPI<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            store: app_state.store.clone(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct AppStateAuthentication {
    pub(crate) client: Client,
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) authenticator: Arc<Authenticator>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateAuthentication {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            client: app_state.client.clone(),
            participant_info: app_state.participant_info.clone(),
            authenticator: app_state.authenticator.clone(),
        }
    }
}

pub(crate) struct AppStateNegotiation<T: Store> {
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) store: Arc<T>,
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) client: Client,
    pub(crate) tx: Sender<NegotiationEvent>,
    pub(crate) validator: Arc<SchemaValidator>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateNegotiation<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            participant_info: app_state.participant_info.clone(),
            store: app_state.store.clone(),
            authenticator: app_state.authenticator.clone(),
            client: app_state.client.clone(),
            tx: app_state.negotiation_events.clone(),
            validator: app_state.validator.clone(),
        }
    }
}

pub(crate) struct AppStateTransfer<T: Store> {
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) store: Arc<T>,
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) client: Client,
    pub(crate) tx: Sender<TransferEvent>,
    pub(crate) validator: Arc<SchemaValidator>,
}

impl<T: Store> FromRef<AppState<T>> for AppStateTransfer<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            participant_info: app_state.participant_info.clone(),
            store: app_state.store.clone(),
            authenticator: app_state.authenticator.clone(),
            client: app_state.client.clone(),
            tx: app_state.transfer_events.clone(),
            validator: app_state.validator.clone(),
        }
    }
}

pub(crate) struct AppStateCatalog<T: Store> {
    pub(crate) store: Arc<T>,
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) client: Client,
    pub(crate) participant_info: ParticipantInfo,
    pub(crate) federation: Arc<HashMap<String, RemoteConnector>>,
    pub(crate) validator: Arc<SchemaValidator>,
}

impl<T: Store> Clone for AppStateCatalog<T> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            authenticator: self.authenticator.clone(),
            client: self.client.clone(),
            participant_info: self.participant_info.clone(),
            federation: self.federation.clone(),
            validator: self.validator.clone(),
        }
    }
}

impl<T: Store> FromRef<AppState<T>> for AppStateCatalog<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            store: app_state.store.clone(),
            authenticator: app_state.authenticator.clone(),
            client: app_state.client.clone(),
            participant_info: app_state.participant_info.clone(),
            federation: app_state.federation.clone(),
            validator: app_state.validator.clone(),
        }
    }
}

pub(crate) struct AppStateReverseProxy<T: Store> {
    pub(crate) store: Arc<T>,
    pub(crate) client: Client,
}

impl<T: Store> FromRef<AppState<T>> for AppStateReverseProxy<T> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        Self {
            store: app_state.store.clone(),
            client: app_state.client.clone(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct KeyPair {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    public_jwk: Jwk,
}

impl KeyPair {
    pub(crate) fn from_ec_pem(private_key_pem: &str) -> anyhow::Result<Self> {
        let encoding_key = EncodingKey::from_ec_pem(private_key_pem.as_bytes())
            .context("Failed to decode private key format")?;
        let jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256)
            .context("Failed to convert encoding key to jwk")?;
        let decoding_key = DecodingKey::from_jwk(&jwk).context("Failed to create decoding key")?;

        Ok(Self {
            encoding_key,
            decoding_key,
            public_jwk: jwk,
        })
    }

    pub(crate) fn public_jwk(&self) -> &Jwk {
        &self.public_jwk
    }

    pub(crate) fn encode<T: Serialize>(&self, claims: T) -> anyhow::Result<String> {
        let header = Header::new(Algorithm::ES256);
        encode(&header, &claims, &self.encoding_key).map_err(anyhow::Error::msg)
    }

    pub(crate) fn encode_with_kid<T: Serialize>(
        &self,
        claims: T,
        kid: String,
    ) -> anyhow::Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(kid);
        encode(&header, &claims, &self.encoding_key).map_err(anyhow::Error::msg)
    }

    pub(crate) fn decode<T>(&self, token: &str) -> anyhow::Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let token_data = {
            let validation = Validation::new(Algorithm::ES256);
            decode::<T>(&token, &self.decoding_key, &validation)?
        };
        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

    #[test]
    fn test_encode_with_kid() {
        let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).expect("valid test key");
        let did = "did:web:party-a";
        let kid = format!("{did}#keys-1");

        let token = key_pair
            .encode_with_kid(json!({ "sub": did }), kid.clone())
            .expect("token encodes");

        let header = jsonwebtoken::decode_header(&token).expect("header decodes");
        assert_eq!(header.kid, Some(kid));
    }
}
