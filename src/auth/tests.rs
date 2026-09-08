use std::{collections::HashMap, sync::Arc};

use serde_json::json;

use crate::{
    auth::{Authenticator, extractor::AuthClaims, model::CredentialData},
    connector::app_state::TokenKeyPair,
    wallet::{KeyPair, store::FileCredentialStore, test_support::SharedResolver},
};

const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

const TEST_TOKEN_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgOLhFoYt4aTJPYhB1
xXg3gfG89YtndEF8nsUw6qc8sLehRANCAARchX5jJbVHUbfxdOYi4EgblpzYpImY
1HlN/B9GVre4HOhDn1TYnpWsX/J6AU5I5v6VxFXqzLU9GW67PG+kxkHp
-----END PRIVATE KEY-----";

fn test_authenticator() -> Authenticator {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    Authenticator::for_test(
        TokenKeyPair::from_ec_pem(TEST_TOKEN_KEY_PEM).expect("valid test key"),
        KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).expect("valid test key"),
        "did:web:party-a",
        Vec::new(),
        Arc::new(SharedResolver::new()),
        Arc::new(FileCredentialStore::new(temp_dir.path().to_path_buf())),
    )
}

#[test]
fn test_did_document_publishes_the_credential_key_only() {
    let published =
        test_authenticator().local_did_document["verificationMethod"][0]["publicKeyJwk"].clone();

    let jwk = |pem| {
        serde_json::to_value(KeyPair::from_ec_pem(pem).expect("valid key").public_jwk())
            .expect("serializes")
    };

    assert_eq!(published, jwk(TEST_PRIVATE_KEY_PEM));
    assert_ne!(published, jwk(TEST_TOKEN_KEY_PEM));
}

fn identity_credential(credential_data: serde_json::Value) -> HashMap<String, CredentialData> {
    HashMap::from([(
        "identity".to_string(),
        CredentialData {
            r#type: "IdentityCredential".to_string(),
            format: "vc+jwt".to_string(),
            credential_data,
            issuer: "did:web:issuer-did-server".to_string(),
        },
    )])
}

#[test]
fn test_derive_access_token_maps_identity_claims() {
    let authenticator = test_authenticator();

    let credentials = identity_credential(json!({
        "email": "ops@autoparts.example",
        "address": { "country": "DE" }
    }));

    let token = authenticator
        .derive_access_token(
            credentials,
            "did:web:party-a".to_string(),
            "did:web:party-b".to_string(),
        )
        .expect("derivation succeeds")
        .expect("credential mapped to a token");

    let claims = authenticator.decode(&token).expect("token decodes");
    assert_eq!(
        claims.data.get("email"),
        Some(&json!("ops@autoparts.example"))
    );
    assert_eq!(claims.data.get("country"), Some(&json!("DE")));
    assert_eq!(claims.data.get("iss"), Some(&json!("did:web:party-a")));
    assert_eq!(claims.data.get("sub"), Some(&json!("did:web:party-b")));
}

#[test]
fn test_derive_access_token_missing_identity_returns_none() {
    let authenticator = test_authenticator();

    let credentials = HashMap::from([(
        "membership".to_string(),
        CredentialData {
            r#type: "MembershipCredential".to_string(),
            format: "vc+jwt".to_string(),
            credential_data: json!({ "email": "ops@autoparts.example" }),
            issuer: "did:web:issuer-did-server".to_string(),
        },
    )]);

    let result = authenticator
        .derive_access_token(
            credentials,
            "did:web:party-a".to_string(),
            "did:web:party-b".to_string(),
        )
        .expect("derivation succeeds");

    assert!(result.is_none());
}

#[test]
fn test_derive_access_token_malformed_identity_returns_none() {
    let authenticator = test_authenticator();

    // No `address`, so the identity credential does not deserialize.
    let credentials = identity_credential(json!({ "email": "ops@autoparts.example" }));

    let result = authenticator
        .derive_access_token(
            credentials,
            "did:web:party-a".to_string(),
            "did:web:party-b".to_string(),
        )
        .expect("derivation succeeds");

    assert!(result.is_none());
}

#[test]
fn test_local_did_document_advertises_wallet_and_dsp_services() {
    let authenticator = test_authenticator();
    let document = authenticator.local_did_document.clone();

    assert_eq!(document["id"], "did:web:party-a");
    let services = document["service"].as_array().expect("service array");

    let endpoint = |r#type: &str| {
        services
            .iter()
            .find(|s| s["type"] == r#type)
            .unwrap_or_else(|| panic!("no {type} entry", type = r#type))["serviceEndpoint"]
            .as_str()
            .expect("string endpoint")
            .to_string()
    };

    assert_eq!(endpoint("CredentialService"), "http://party-a/api/credentials/v1");
    assert_eq!(endpoint("IssuerService"), "http://party-a/api/issuance/v1");
    assert_eq!(endpoint("CatalogService"), "http://party-a/api/2025/1/catalog");

    // A peer strips the version path off this to get our <root>, so the entry has to
    // point at the version endpoint and nothing else.
    assert_eq!(
        endpoint("DataService"),
        "http://party-a/.well-known/dspace-version"
    );
    assert_eq!(
        services
            .iter()
            .find(|s| s["type"] == "DataService")
            .expect("DataService entry")["id"],
        "did:web:party-a#data-service"
    );
}

#[test]
fn test_auth_claims_expiration() {
    let fresh = AuthClaims::default();
    assert!(!fresh.is_expired());

    let mut expired = AuthClaims::default();
    expired.data.insert("exp".into(), json!(0));
    assert!(expired.is_expired());
}

#[test]
fn test_auth_claims_serialization_round_trip() {
    let mut claims = AuthClaims::default();
    claims.data.insert("sub".into(), json!("did:web:party-b"));

    let encoded = serde_json::to_string(&claims).expect("claims serialize");
    let decoded: AuthClaims = serde_json::from_str(&encoded).expect("claims deserialize");

    assert_eq!(decoded.data.get("sub"), Some(&json!("did:web:party-b")));
    assert_eq!(decoded.data.get("jti"), claims.data.get("jti"));
}
