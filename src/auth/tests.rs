use std::{collections::HashMap, sync::Arc};

use reqwest::Client;
use serde_json::json;

use crate::{
    auth::{
        Authenticator, backend::AuthBackend, backends::native::NativeBackend,
        extractor::AuthClaims, model::CredentialData, model::DidDocument,
    },
    connector::app_state::KeyPair,
    dcp::test_support::SharedResolver,
};

const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZSL1BoLkGm8MArlQ
m60MC/g5J+38YsAycU1hukHQg3ehRANCAAQNLDb617BSV/8pn5Z/exH3sS2rMkes
5ZBcTa62LI1MRsxdz5kut+l2YWH79puf51LHNRSr35RT+smF3DcFjgg3
-----END PRIVATE KEY-----";

fn test_authenticator() -> Authenticator {
    let backend: Arc<dyn AuthBackend> = Arc::new(NativeBackend::new(
        KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).expect("valid test key"),
        "did:web:party-a".to_string(),
        "did:web:party-a#keys-1".to_string(),
        Vec::new(),
        Arc::new(SharedResolver::new()),
        Client::new(),
    ));
    let key_pair = KeyPair::from_ec_pem(TEST_PRIVATE_KEY_PEM).expect("valid test key");
    Authenticator::new(backend, key_pair)
}

fn identity_credential(credential_data: serde_json::Value) -> HashMap<String, CredentialData> {
    HashMap::from([(
        "identity".to_string(),
        CredentialData {
            r#type: "IdentityCredential".to_string(),
            format: "dc+sd-jwt".to_string(),
            credential_data,
            issuer: "did:web:issuer-did-server".to_string(),
        },
    )])
}

#[test]
fn test_native_auth_derive_access_token_maps_identity_claims() {
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
fn test_native_auth_derive_access_token_missing_identity_returns_none() {
    let authenticator = test_authenticator();

    let credentials = HashMap::from([(
        "membership".to_string(),
        CredentialData {
            r#type: "MembershipCredential".to_string(),
            format: "dc+sd-jwt".to_string(),
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
fn test_native_auth_derive_access_token_malformed_identity_returns_none() {
    let authenticator = test_authenticator();

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
fn test_native_auth_did_document_decoding_key_matches_authentication() {
    let document: DidDocument = serde_json::from_value(json!({
        "id": "did:web:party",
        "verificationMethod": [{
            "id": "did:web:party#key-1",
            "type": "JsonWebKey2020",
            "controller": "did:web:party",
            "publicKeyJwk": {
                "kty": "EC",
                "crv": "P-256",
                "x": "G0RINBiF-oQUD3d5DGnegQuXenI29JDaMGoMvioKRBM",
                "y": "ed3eFGs2pEtrp7vAZ7BLcbrUtpKkYWAT2JPUQK4lN4E"
            }
        }],
        "authentication": ["did:web:party#key-1"]
    }))
    .expect("did document deserializes");

    assert!(document.decoding_key().is_ok());
}

#[test]
fn test_native_auth_did_document_decoding_key_without_match_fails() {
    let document: DidDocument = serde_json::from_value(json!({
        "id": "did:web:party",
        "verificationMethod": [{
            "id": "did:web:party#key-1",
            "type": "JsonWebKey2020",
            "controller": "did:web:party",
            "publicKeyJwk": {
                "kty": "EC",
                "crv": "P-256",
                "x": "G0RINBiF-oQUD3d5DGnegQuXenI29JDaMGoMvioKRBM",
                "y": "ed3eFGs2pEtrp7vAZ7BLcbrUtpKkYWAT2JPUQK4lN4E"
            }
        }],
        "authentication": ["did:web:party#missing"]
    }))
    .expect("did document deserializes");

    assert!(document.decoding_key().is_err());
}

#[test]
fn test_native_auth_claims_expiration() {
    let fresh = AuthClaims::default();
    assert!(!fresh.is_expired());

    let mut expired = AuthClaims::default();
    expired.data.insert("exp".into(), json!(0));
    assert!(expired.is_expired());
}

#[test]
fn test_native_auth_claims_serialization_round_trip() {
    let mut claims = AuthClaims::default();
    claims.data.insert("sub".into(), json!("did:web:party-b"));

    let encoded = serde_json::to_string(&claims).expect("claims serialize");
    let decoded: AuthClaims = serde_json::from_str(&encoded).expect("claims deserialize");

    assert_eq!(decoded.data.get("sub"), Some(&json!("did:web:party-b")));
    assert_eq!(decoded.data.get("jti"), claims.data.get("jti"));
}
