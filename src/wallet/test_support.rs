use std::{collections::HashMap, net::SocketAddr, sync::RwLock};

use async_trait::async_trait;
use axum::Router;
use jsonwebtoken::{Algorithm, EncodingKey, jwk::Jwk};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::oneshot};

use crate::{
    shared::DidDocument,
    shared::KeyPair,
    wallet::{dcp::si_token::build_si_token, did::DidResolver},
};

pub(crate) const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

pub(crate) const ISSUER_DID: &str = "did:web:party-a";
pub(crate) const HOLDER_DID: &str = "did:web:party-b";

pub(crate) fn test_key_pair() -> KeyPair {
    KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).unwrap()
}

pub(crate) fn kid(did: &str) -> String {
    format!("{did}#keys-1")
}

pub(crate) fn public_jwk() -> Value {
    let encoding_key = EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
    let jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256).unwrap();
    serde_json::to_value(&jwk).unwrap()
}

pub(crate) fn make_did_document(
    did: &str,
    credential_service_endpoint: Option<&str>,
    issuer_service_endpoint: Option<&str>,
) -> Value {
    let mut services = Vec::new();
    if let Some(ep) = credential_service_endpoint {
        services.push(json!({
            "id": format!("{did}#credential-service"),
            "type": "CredentialService",
            "serviceEndpoint": ep
        }));
    }
    if let Some(ep) = issuer_service_endpoint {
        services.push(json!({
            "id": format!("{did}#issuer-service"),
            "type": "IssuerService",
            "serviceEndpoint": ep
        }));
    }

    json!({
        "id": did,
        "verificationMethod": [{
            "id": kid(did),
            "type": "JsonWebKey2020",
            "controller": did,
            "publicKeyJwk": public_jwk()
        }],
        "authentication": [kid(did)],
        "assertionMethod": [kid(did)],
        "capabilityInvocation": [kid(did)],
        "service": services
    })
}

fn did_document(did: &str) -> Value {
    make_did_document(
        did,
        Some("http://127.0.0.1:0/api/credentials/v1"),
        Some("http://127.0.0.1:0/api/issuance/v1"),
    )
}

pub(crate) struct StaticResolver {
    documents: HashMap<String, Value>,
}

#[async_trait]
impl DidResolver for StaticResolver {
    async fn resolve(&self, did: &str) -> anyhow::Result<DidDocument> {
        let document = self
            .documents
            .get(did)
            .ok_or_else(|| anyhow::anyhow!("unknown did {did}"))?;

        Ok(serde_json::from_value(document.clone())?)
    }
}

pub(crate) fn static_resolver() -> StaticResolver {
    let mut documents = HashMap::new();
    documents.insert(ISSUER_DID.to_string(), did_document(ISSUER_DID));
    documents.insert(HOLDER_DID.to_string(), did_document(HOLDER_DID));
    StaticResolver { documents }
}

pub(crate) fn holder_si_token(aud: &str) -> String {
    build_si_token(
        &test_key_pair(),
        HOLDER_DID.to_string(),
        aud.to_string(),
        kid(HOLDER_DID),
        None,
    )
    .unwrap()
}

pub(crate) fn issuer_si_token(aud: &str) -> String {
    build_si_token(
        &test_key_pair(),
        ISSUER_DID.to_string(),
        aud.to_string(),
        kid(ISSUER_DID),
        None,
    )
    .unwrap()
}

pub(crate) struct SharedResolver {
    documents: RwLock<HashMap<String, Value>>,
}

#[async_trait]
impl DidResolver for SharedResolver {
    async fn resolve(&self, did: &str) -> anyhow::Result<DidDocument> {
        let docs = self.documents.read().unwrap();
        let document = docs
            .get(did)
            .ok_or_else(|| anyhow::anyhow!("unknown did {did}"))?;

        Ok(serde_json::from_value(document.clone())?)
    }
}

impl SharedResolver {
    pub(crate) fn new() -> Self {
        Self {
            documents: RwLock::new(HashMap::new()),
        }
    }

    pub(crate) fn register(&self, did: &str, document: Value) {
        self.documents
            .write()
            .unwrap()
            .insert(did.to_string(), document);
    }
}

pub(crate) struct TestServer {
    pub(crate) addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl TestServer {
    pub(crate) async fn spawn(router: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}
