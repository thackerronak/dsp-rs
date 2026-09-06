use std::{process::Command, time::Duration};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

fn form_encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

const COMPOSE_FILE: &str = "docker-compose/docker-compose.yml";
const PARTY_A: &str = "http://localhost:13000";
const PARTY_B: &str = "http://localhost:23000";
const A_DID: &str = "did:web:party-a-connector%3A3000";
const DATASET_ID: &str = "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57";

fn docker(args: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(args)
        .output()
        .expect("failed to run docker; is it installed and running?")
}

fn compose(args: &[&str]) -> std::process::Output {
    let mut full = vec!["compose", "-f", COMPOSE_FILE];
    full.extend_from_slice(args);
    docker(&full)
}

fn ensure_image() {
    if docker(&["image", "inspect", "connector:latest"])
        .status
        .success()
    {
        return;
    }
    let out = docker(&["build", "-t", "connector:latest", "-f", "Dockerfile", "."]);
    assert!(
        out.status.success(),
        "docker build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Stack;

impl Stack {
    fn up() -> Self {
        ensure_image();
        let _ = compose(&["down"]);
        let out = compose(&["up", "-d"]);
        assert!(
            out.status.success(),
            "docker compose up failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Stack
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = compose(&["down"]);
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap()
}

async fn wait_healthy(client: &reqwest::Client, base: &str) {
    for _ in 0..90 {
        if let Ok(resp) = client
            .get(format!("{base}/.well-known/did.json"))
            .send()
            .await
            && resp.status().is_success()
        {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("{base} did not become healthy in time");
}

async fn held_identity_credentials(client: &reqwest::Client, base: &str) -> usize {
    match client
        .get(format!("{base}/api/credentials/v1/credentials"))
        .send()
        .await
    {
        Ok(resp) => resp
            .json::<Vec<Value>>()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|c| c["credentialType"] == "identity_credential")
            .count(),
        Err(_) => 0,
    }
}

async fn ensure_credential(client: &reqwest::Client, holder_base: &str, issuer_did: &str) {
    if held_identity_credentials(client, holder_base).await > 0 {
        return;
    }

    let status = client
        .post(format!("{holder_base}/api/credentials/v1/request"))
        .json(&json!({ "issuerDid": issuer_did }))
        .send()
        .await
        .expect("credential request")
        .status();
    assert!(
        status.is_success(),
        "credential request to {holder_base} failed: {status}"
    );

    for _ in 0..20 {
        if held_identity_credentials(client, holder_base).await > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("{holder_base} holds no identity_credential after issuance");
}

#[tokio::test]
#[ignore = "requires Docker; run: cargo test --test e2e_docker -- --ignored --nocapture"]
async fn e2e_native_dcp_catalog_over_docker() {
    let _stack = Stack::up();
    let client = client();

    wait_healthy(&client, PARTY_A).await;
    wait_healthy(&client, PARTY_B).await;

    ensure_credential(&client, PARTY_B, A_DID).await;

    let sts_body = format!(
        "grant_type=client_credentials&client_id=dsp-client&client_secret=dsp-secret&audience={}",
        form_encode(A_DID)
    );
    let sts: Value = client
        .post(format!("{PARTY_B}/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(sts_body)
        .send()
        .await
        .expect("sts request")
        .json()
        .await
        .expect("sts json");
    let si_token = sts["access_token"]
        .as_str()
        .expect("sts access_token")
        .to_string();

    let auth = client
        .post(format!("{PARTY_A}/auth/token"))
        .bearer_auth(&si_token)
        .send()
        .await
        .expect("auth/token request");
    assert_eq!(
        auth.status(),
        reqwest::StatusCode::OK,
        "/auth/token should succeed"
    );
    let auth: Value = auth.json().await.expect("auth json");
    assert_eq!(auth["token_type"], "Bearer");
    let access_token = auth["access_token"]
        .as_str()
        .expect("dsp access_token")
        .to_string();

    let catalog = client
        .post(format!("{PARTY_A}/api/2025/1/catalog/request"))
        .bearer_auth(&access_token)
        .json(&json!({
            "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
            "@type": "CatalogRequest"
        }))
        .send()
        .await
        .expect("catalog request");
    assert_eq!(
        catalog.status(),
        reqwest::StatusCode::OK,
        "catalog request should succeed"
    );
    let catalog: Value = catalog.json().await.expect("catalog json");

    let datasets = catalog["dataset"].as_array().cloned().unwrap_or_default();
    assert!(
        datasets.iter().any(|d| d["@id"] == DATASET_ID),
        "catalog should contain the seeded dataset {DATASET_ID}, got: {catalog}"
    );
}
