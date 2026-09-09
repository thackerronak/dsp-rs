use std::{sync::Arc, time::Duration};

use axum::{Router, http::StatusCode};
use jsonwebtoken::dangerous::insecure_decode;
use reqwest::Client;
use serde_json::Value;

use crate::wallet::{
    dcp::{
        holder::{self, HolderState},
        issuer::{self, IssuerState},
    },
    store::{CredentialStore, FileCredentialStore, StoredCredential},
    test_support::{
        HOLDER_DID, ISSUER_DID, SharedResolver, TestServer, kid, make_did_document, test_key_pair,
    },
};

#[tokio::test]
async fn test_issuance_and_delivery_integration() {
    let resolver = Arc::new(SharedResolver::new());
    let client = Client::builder().no_proxy().build().unwrap();

    // 1. Party A: Issuer
    let issuer_state = IssuerState::new(
        test_key_pair(),
        ISSUER_DID.to_string(),
        kid(ISSUER_DID),
        client.clone(),
        resolver.clone(),
    );
    let issuer_app = Router::new().nest("/api/issuance/v1", issuer::router(issuer_state));
    let issuer_server = TestServer::spawn(issuer_app).await;
    let issuer_endpoint = issuer_server.url("/api/issuance/v1");

    // 2. Party B: Holder with FileCredentialStore
    let temp_dir = tempfile::tempdir().unwrap();
    let credential_store = Arc::new(FileCredentialStore::new(temp_dir.path().to_path_buf()));
    let holder_state = HolderState::new(
        test_key_pair(),
        HOLDER_DID.to_string(),
        kid(HOLDER_DID),
        client.clone(),
        resolver.clone(),
        credential_store.clone(),
    );
    let holder_app = Router::new().nest("/api/credentials/v1", holder::router(holder_state));
    let holder_server = TestServer::spawn(holder_app).await;
    let holder_endpoint = holder_server.url("/api/credentials/v1");

    // 3. Register live endpoints in shared DID resolver
    resolver.register(
        ISSUER_DID,
        make_did_document(ISSUER_DID, None, Some(&issuer_endpoint)),
    );
    resolver.register(
        HOLDER_DID,
        make_did_document(HOLDER_DID, Some(&holder_endpoint), None),
    );

    // 4. Trigger Party A to send offer to Party B
    let offer_trigger_resp = client
        .post(issuer_server.url("/api/issuance/v1/offer"))
        .json(&serde_json::json!({
            "holderDid": HOLDER_DID
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(offer_trigger_resp.status(), StatusCode::ACCEPTED);

    // 5. Poll Party B's FileCredentialStore until delivered credential is saved
    let mut stored_credentials = Vec::new();
    for _ in 0..50 {
        let list = credential_store.list().await.unwrap();
        if !list.is_empty() {
            stored_credentials = list;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(
        stored_credentials.len(),
        1,
        "Expected exactly 1 stored credential after issuance flow"
    );

    let credential = &stored_credentials[0];
    assert!(
        credential.id.starts_with(&format!("{ISSUER_DID}#")),
        "Credential id should be prefixed with issuer DID"
    );
    assert_eq!(credential.credential_type, "identity_credential");
    assert_eq!(credential.format, "VC1_0_JWT");
    assert_eq!(credential.issuer, ISSUER_DID);

    // 6. Verify signed JWT payload contents
    let decoded = insecure_decode::<Value>(&credential.payload).unwrap();
    let claims = decoded.claims;
    assert_eq!(claims["iss"], ISSUER_DID);
    assert_eq!(claims["sub"], HOLDER_DID);
    let vc = &claims["vc"];
    let types = vc["type"].as_array().expect("vc.type should be an array");
    assert!(types.iter().any(|t| t == "identity_credential"));
    assert_eq!(vc["credentialSubject"]["address"]["country"], "DE");

    // 7. Verify Party B's REST endpoint GET /api/credentials/v1/credentials
    let rest_list_resp = client
        .get(holder_server.url("/api/credentials/v1/credentials"))
        .send()
        .await
        .unwrap();
    assert_eq!(rest_list_resp.status(), StatusCode::OK);
    let list_body: Vec<StoredCredential> = rest_list_resp.json().await.unwrap();
    assert_eq!(list_body.len(), 1);
    assert_eq!(list_body[0], *credential);
}
