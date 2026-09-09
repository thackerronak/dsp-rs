use std::{collections::HashMap, sync::Arc};

use axum::Router;
use jsonwebtoken::dangerous::insecure_decode;
use reqwest::Client;
use serde_json::json;

use crate::{
    auth::{Authenticator, extractor::AuthClaims, model::CredentialData},
    connector::app_state::TokenKeyPair,
    wallet::{
        KeyPair,
        dcp::{
            holder::{self, HolderState},
            issuer::mint_identity_credential,
            si_token::{SiClaims, build_si_token},
        },
        store::{CredentialStore, FileCredentialStore, StoredCredential},
        test_support::{
            HOLDER_DID, ISSUER_DID, SharedResolver, TestServer, kid, make_did_document,
            test_key_pair,
        },
    },
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

    assert_eq!(
        endpoint("CredentialService"),
        "http://party-a/api/credentials/v1"
    );
    assert_eq!(endpoint("IssuerService"), "http://party-a/api/issuance/v1");
    assert_eq!(
        endpoint("CatalogService"),
        "http://party-a/api/2025/1/catalog"
    );

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

/// The verifier is party A, which in the demo topology also issued the peer's credential.
const VERIFIER_DID: &str = ISSUER_DID;

/// Stands up a Credential Service for `HOLDER_DID` holding one identity credential, and a
/// verifier that trusts the credential's issuer. Returns the verifier and the live server,
/// which must stay alive for the duration of the test.
async fn peer_with_credential_service(
    resolver: Arc<SharedResolver>,
    subject: serde_json::Value,
) -> (
    Authenticator,
    TestServer,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let client = Client::builder().no_proxy().build().expect("test client");
    let key_pair = test_key_pair();

    let peer_dir = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(FileCredentialStore::new(peer_dir.path().to_path_buf()));

    let vc = mint_identity_credential(&key_pair, ISSUER_DID, &kid(ISSUER_DID), HOLDER_DID, subject)
        .expect("mints a credential");

    store
        .store(&StoredCredential {
            id: format!("{ISSUER_DID}#pid-1"),
            credential_type: "identity_credential".to_string(),
            format: "vc+jwt".to_string(),
            issuer: ISSUER_DID.to_string(),
            payload: vc,
        })
        .await
        .expect("stores the credential");

    let holder_state = HolderState::new(
        key_pair.clone(),
        HOLDER_DID.to_string(),
        kid(HOLDER_DID),
        client,
        resolver.clone(),
        store,
    );
    let server =
        TestServer::spawn(Router::new().nest("/api/credentials/v1", holder::router(holder_state)))
            .await;

    resolver.register(
        HOLDER_DID,
        make_did_document(HOLDER_DID, Some(&server.url("/api/credentials/v1")), None),
    );
    resolver.register(VERIFIER_DID, make_did_document(VERIFIER_DID, None, None));

    let verifier_dir = tempfile::tempdir().expect("temp dir");
    let verifier = Authenticator::for_test(
        TokenKeyPair::from_ec_pem(TEST_TOKEN_KEY_PEM).expect("valid test key"),
        key_pair,
        VERIFIER_DID,
        vec![ISSUER_DID.to_string()],
        resolver,
        Arc::new(FileCredentialStore::new(verifier_dir.path().to_path_buf())),
    );

    (verifier, server, peer_dir, verifier_dir)
}

/// An EDC peer puts its Self-Issued ID Token straight on the DSP request rather than
/// asking us to mint an access token first, so the verifier has to run the DCP pull inline.
#[tokio::test]
async fn test_authenticate_accepts_a_peer_self_issued_token() {
    let resolver = Arc::new(SharedResolver::new());
    let (verifier, _server, _peer_dir, _verifier_dir) = peer_with_credential_service(
        resolver,
        json!({ "email": "ops@example.com", "address": { "country": "DE" } }),
    )
    .await;

    // The `token` claim is the access token the peer minted for its own Credential Service.
    let key_pair = test_key_pair();
    let credential_service_token = build_si_token(
        &key_pair,
        HOLDER_DID.to_string(),
        HOLDER_DID.to_string(),
        kid(HOLDER_DID),
        None,
    )
    .expect("mints the inner token");

    let si_token = build_si_token(
        &key_pair,
        HOLDER_DID.to_string(),
        VERIFIER_DID.to_string(),
        kid(HOLDER_DID),
        Some(credential_service_token),
    )
    .expect("mints the SI token");

    let claims = verifier
        .authenticate(&si_token, VERIFIER_DID.to_string())
        .await
        .expect("authenticates the peer");

    assert_eq!(claims.data["email"], "ops@example.com");
    assert_eq!(claims.data["country"], "DE");
    assert_eq!(claims.data["iss"], VERIFIER_DID);
    assert_eq!(claims.data["sub"], HOLDER_DID);
}

/// A DSP access token minted by `POST /auth/token` keeps working, so peers that ask for
/// one up front are unaffected.
#[tokio::test]
async fn test_authenticate_still_accepts_an_access_token_we_minted() {
    let resolver = Arc::new(SharedResolver::new());
    let (verifier, _server, _peer_dir, _verifier_dir) = peer_with_credential_service(
        resolver,
        json!({ "email": "ops@example.com", "address": { "country": "DE" } }),
    )
    .await;

    let access_token = verifier
        .derive_access_token(
            identity_credential(json!({
                "email": "ops@example.com",
                "address": { "country": "DE" }
            })),
            VERIFIER_DID.to_string(),
            HOLDER_DID.to_string(),
        )
        .expect("derives a token")
        .expect("credential maps to claims");

    let claims = verifier
        .authenticate(&access_token, VERIFIER_DID.to_string())
        .await
        .expect("authenticates our own token");

    assert_eq!(claims.data["email"], "ops@example.com");
    assert_eq!(claims.data["sub"], HOLDER_DID);
}

#[tokio::test]
async fn test_authenticate_rejects_a_token_that_is_neither() {
    let resolver = Arc::new(SharedResolver::new());
    let (verifier, _server, _peer_dir, _verifier_dir) = peer_with_credential_service(
        resolver,
        json!({ "email": "ops@example.com", "address": { "country": "DE" } }),
    )
    .await;

    assert!(
        verifier
            .authenticate("not-a-jwt", VERIFIER_DID.to_string())
            .await
            .is_err()
    );
}

/// Outbound side of the same protocol: the token we hand a peer is a Self-Issued ID Token
/// carrying an access token for our own Credential Service, not an opaque access token
/// fetched from the peer.
#[tokio::test]
async fn test_get_token_mints_a_self_issued_token_with_an_embedded_access_token() {
    let token = test_authenticator()
        .get_token("http://party-b", "did:web:party-b".to_string())
        .await
        .expect("mints a token");

    let claims = insecure_decode::<SiClaims>(&token).expect("decodes").claims;
    assert_eq!(claims.iss, "did:web:party-a");
    assert_eq!(claims.sub, "did:web:party-a");
    assert_eq!(claims.aud, "did:web:party-b");

    let embedded = claims.token.expect("carries a `token` claim");
    let inner = insecure_decode::<SiClaims>(&embedded)
        .expect("decodes")
        .claims;
    assert_eq!(inner.iss, "did:web:party-a");
    assert_eq!(inner.aud, "did:web:party-a");
}
