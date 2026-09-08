use std::{collections::HashMap, sync::Arc};

use serde_json::json;

use crate::{
    auth::{Authenticator, extractor::AuthClaims, model::CredentialData},
    shared::KeyPair,
    wallet::{store::FileCredentialStore, test_support::SharedResolver},
};

const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

fn test_authenticator() -> Authenticator {
    let temp_dir = tempfile::tempdir().expect("temp dir");

    Authenticator::for_test(
        KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).expect("valid test key"),
        "did:web:party-a",
        Vec::new(),
        Arc::new(SharedResolver::new()),
        Arc::new(FileCredentialStore::new(temp_dir.path().to_path_buf())),
    )
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
fn test_local_did_document_advertises_wallet_services() {
    let authenticator = test_authenticator();
    let document = authenticator.local_did_document.clone();

    assert_eq!(document["id"], "did:web:party-a");
    let services = document["service"].as_array().expect("service array");
    let types: Vec<&str> = services
        .iter()
        .filter_map(|s| s["type"].as_str())
        .collect();
    assert!(types.contains(&"CredentialService"));
    assert!(types.contains(&"IssuerService"));
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
