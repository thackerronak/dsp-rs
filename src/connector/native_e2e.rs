use std::path::Path;

use reqwest::Client;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::{
    auth::backends::native::NativeBackend,
    connector::{build_app, utils::derive_did_web},
    dcp::{
        issuer::mint_identity_credential,
        test_support::{TEST_PRIVATE_KEY_PEM, kid, test_key_pair},
    },
    store::{credential_store::StoredCredential, file_store::FileStore},
};

use super::app_state::AppState;

const IDENTITY_SCOPE_TYPE: &str = "identity_credential";

fn native_config(
    external_address: &str,
    credential_store_path: &str,
    allowed_issuers: Vec<String>,
) -> Value {
    json!({
        "participant_info": {
            "id": external_address,
            "external_address": external_address
        },
        "private_key_pem": TEST_PRIVATE_KEY_PEM,
        "issuer_url": "",
        "verifier_url": "",
        "wallet_url": "",
        "wallet_id": "",
        "allowed_issuers": allowed_issuers,
        "federation": {},
        "auth": {
            "backend": "native",
            "native": {
                "credential_store_path": credential_store_path,
                "credential_service_path": "/api/credentials/v1",
                "issuance_service_path": "/api/issuance/v1",
                "issuer_enabled": true,
                "sts_client_id": "dsp-client",
                "sts_client_secret": "dsp-secret"
            }
        }
    })
}

fn seed_dataset(data_dir: &Path, dataset_id: &str) {
    let datasets_dir = data_dir.join("datasets");
    std::fs::create_dir_all(&datasets_dir).unwrap();

    let descriptor = json!({
        "dataset": {
            "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@id": dataset_id,
            "@type": "Dataset",
            "hasPolicy": [{
                "@type": "Offer",
                "@id": "urn:uuid:offer-e2e-1",
                "permission": [{ "action": "use" }]
            }],
            "distribution": [{
                "@type": "Distribution",
                "format": "HttpData-PULL",
                "accessService": {
                    "@id": "urn:uuid:access-e2e-1",
                    "@type": "DataService",
                    "endpointURL": "http://provider.example/connector"
                }
            }]
        },
        "remoteAddress": {
            "url": "http://localhost:4000/api",
            "passHeaders": true
        }
    });

    std::fs::write(
        datasets_dir.join("test-dataset.json"),
        serde_json::to_vec_pretty(&descriptor).unwrap(),
    )
    .unwrap();
}

struct Party {
    state: AppState<FileStore>,
    address: String,
    did: String,
}

async fn spawn_party(data_dir: &Path, credential_store_path: &str, trust_self: bool) -> Party {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let address = format!("http://{addr}");
    let did = derive_did_web(&address).unwrap();

    let allowed_issuers = if trust_self {
        vec![did.clone()]
    } else {
        Vec::new()
    };
    let config = native_config(&address, credential_store_path, allowed_issuers);

    let store = FileStore::new(data_dir.to_path_buf());
    let (state, _rx_n, _rx_t) = AppState::from_config_json(config, store).await.unwrap();

    let app = build_app(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Party {
        state,
        address,
        did,
    }
}

fn native_backend(party: &Party) -> &NativeBackend {
    party
        .state
        .authenticator
        .backend()
        .as_any()
        .downcast_ref::<NativeBackend>()
        .expect("native backend")
}

async fn seed_credential(holder: &Party, issuer_did: &str) {
    let key_pair = test_key_pair();
    let vc = mint_identity_credential(
        &key_pair,
        issuer_did,
        &kid(issuer_did),
        &holder.did,
        json!({
            "email": "ops@party.example",
            "address": { "country": "DE" }
        }),
    )
    .unwrap();

    let store = native_backend(holder)
        .credential_store()
        .expect("holder credential store");

    store
        .store(&StoredCredential {
            id: format!("{issuer_did}#seeded"),
            credential_type: IDENTITY_SCOPE_TYPE.to_string(),
            format: "vc+jwt".to_string(),
            issuer: issuer_did.to_string(),
            payload: vc,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_connector_startup_native() {
    let data_dir = tempfile::tempdir().unwrap();
    let credential_dir = tempfile::tempdir().unwrap();

    let party = spawn_party(
        data_dir.path(),
        credential_dir.path().to_str().unwrap(),
        false,
    )
    .await;

    let client = Client::builder().no_proxy().build().unwrap();

    let did_doc: Value = client
        .get(format!("{}/.well-known/did.json", party.address))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(did_doc["id"], party.did);
    let types: Vec<&str> = did_doc["service"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|service| service["type"].as_str())
        .collect();
    assert!(types.contains(&"CredentialService"));
    assert!(types.contains(&"IssuerService"));

    let unauthorized = client
        .post(format!("{}/auth/token", party.address))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_e2e_native_dcp_catalog() {
    let a_data = tempfile::tempdir().unwrap();
    let a_creds = tempfile::tempdir().unwrap();
    let b_data = tempfile::tempdir().unwrap();
    let b_creds = tempfile::tempdir().unwrap();

    let dataset_id = "urn:uuid:test-dataset-e2e";
    seed_dataset(a_data.path(), dataset_id);

    let party_a = spawn_party(a_data.path(), a_creds.path().to_str().unwrap(), true).await;
    let party_b = spawn_party(b_data.path(), b_creds.path().to_str().unwrap(), true).await;

    // Bilateral issuance seeding: each party holds the identity credential issued by the party
    // that will verify it (Task 2.4 flow, seeded directly here for determinism).
    seed_credential(&party_b, &party_a.did).await;

    let client = Client::builder().no_proxy().build().unwrap();

    let access_token = party_b
        .state
        .authenticator
        .get_token(&client, &party_a.address, party_a.did.clone())
        .await
        .expect("native auth token exchange succeeds");

    // Party B requests Party A's catalog with the obtained token.
    let response = client
        .post(format!("{}/api/2025/1/catalog/request", party_a.address))
        .bearer_auth(&access_token)
        .json(&json!({
            "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": "CatalogRequest"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let catalog: Value = response.json().await.unwrap();
    let body = serde_json::to_string(&catalog).unwrap();
    assert!(
        body.contains(dataset_id),
        "catalog response should contain the seeded dataset, got: {body}"
    );
}
