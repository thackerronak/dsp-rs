//! End-to-end test over the Docker demo stack.
//!
//! This walks exactly what `docker-compose/SETUP.md` and `USAGE.md` document, against
//! the real `waltid/issuer-api2` container rather than a mock: a credential is issued
//! over OID4VCI, presented over DCP, and then used to negotiate a contract and pull a
//! dataset.
//!
//! Ignored by default — it needs Docker, builds an image, and takes minutes. Run it with:
//!
//! ```sh
//! cargo test --test e2e_docker -- --ignored --nocapture
//! ```
//!
//! It is deliberately **one** test function. The steps share one stack and one set of
//! host ports, so splitting them into separate `#[test]`s would let cargo run them in
//! parallel against the same containers. It also **tears the stack down and deletes the
//! seeded credentials**, so do not run it against a stack you are using by hand.

use std::{path::Path, process::Command, time::Duration};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

const COMPOSE_FILE: &str = "docker-compose/docker-compose.yml";
const ISSUER: &str = "http://localhost:7002";
const PARTY_A: &str = "http://localhost:13000";
const PARTY_B: &str = "http://localhost:23000";
const A_DID: &str = "did:web:party-a-connector%3A3000";
const B_DID: &str = "did:web:party-b-connector%3A3000";
const ISSUER_DID: &str = "did:web:issuer-did-server";
const DATASET_ID: &str = "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57";
const OFFER_ID: &str = "urn:uuid:2828282:3dd1add8-4d2d-569e-d634-8394a8836a88";
/// The profile that issues a W3C JWT-VC. The bundled SD-JWT profile is not readable by
/// this connector's verifier.
const PROFILE: &str = "identityCredentialJwtVc";
/// What the issuer advertises as the credential configuration's `scope`, and therefore
/// the credential type a DCP presentation query matches on.
const CREDENTIAL_TYPE: &str = "identity_credential";

macro_rules! step {
    ($($arg:tt)*) => { println!("\n=== {} ===", format!($($arg)*)) };
}

// ---------------------------------------------------------------------------
// Stack lifecycle
// ---------------------------------------------------------------------------

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

/// Always rebuild. Docker's layer cache makes this near-instant when nothing changed,
/// and reusing a stale image would test a binary that no longer matches the source.
fn build_image() {
    step!("building connector:latest");
    let out = docker(&["build", "-t", "connector:latest", "-f", "Dockerfile", "."]);
    assert!(
        out.status.success(),
        "docker build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Start from a cold stack: no credentials and no synced catalog, so the test exercises
/// issuance and discovery rather than leftovers from a previous run.
fn clear_state() {
    for party in ["party-a", "party-b"] {
        for dir in ["credentials", "datasets/federated"] {
            let path = format!("docker-compose/{party}/connector/data/{dir}");
            if Path::new(&path).exists() {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
}

struct Stack;

impl Stack {
    fn up() -> Self {
        build_image();
        let _ = compose(&["down"]);
        clear_state();

        step!("docker compose up");
        let out = compose(&["up", "-d"]);
        assert!(
            out.status.success(),
            "docker compose up failed:\n{}",
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap()
}

fn form_encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// Poll `probe` once a second until it yields a value, then return it.
async fn wait_for<T, F, Fut>(what: &str, timeout: Duration, mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + timeout;
    let mut attempts = 0u32;
    loop {
        if let Some(value) = probe().await {
            return value;
        }
        attempts += 1;
        assert!(
            std::time::Instant::now() < deadline,
            "timed out after {timeout:?} ({attempts} attempts) waiting for {what}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn get_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

/// The demo depends on this image advertising a `jwt_vc_json` credential configuration.
/// If the issuer rejects the HOCON, the configuration silently disappears from its
/// metadata rather than failing at startup — so assert on it directly.
async fn assert_issuer_advertises_jwt_vc(client: &reqwest::Client) {
    let metadata = wait_for(
        "issuer credential metadata",
        Duration::from_secs(120),
        || async {
            get_json(
                client,
                &format!("{ISSUER}/.well-known/openid-credential-issuer/openid4vci"),
            )
            .await
        },
    )
    .await;

    let configurations = &metadata["credential_configurations_supported"];
    let jwt_vc = &configurations["IdentityCredential_jwt_vc_json"];
    assert!(
        jwt_vc.is_object(),
        "issuer does not advertise IdentityCredential_jwt_vc_json — it likely rejected \
         docker-compose/issuer/config/. Advertised: {configurations:#}"
    );
    assert_eq!(jwt_vc["format"], "jwt_vc_json");
    assert_eq!(
        jwt_vc["scope"], CREDENTIAL_TYPE,
        "the advertised scope is the DCP credential type the verifier queries for"
    );
    println!("issuer advertises IdentityCredential_jwt_vc_json (format jwt_vc_json)");
}

/// Ask the issuer for a pre-authorized offer, then have the connector redeem it itself.
async fn seed_credential(client: &reqwest::Client, holder: &str, expected_holder_did: &str) {
    let offer: Value = client
        .post(format!("{ISSUER}/issuer2/credential-offers"))
        .json(&json!({ "profileId": PROFILE, "authMethod": "PRE_AUTHORIZED" }))
        .send()
        .await
        .expect("credential-offers request")
        .json()
        .await
        .expect("credential-offers json");

    let offer_url = offer["credentialOffer"]
        .as_str()
        .unwrap_or_else(|| panic!("issuer returned no credentialOffer: {offer:#}"));

    let response = client
        .post(format!("{holder}/api-internal/credentials/redeem"))
        .json(&json!({ "offerUrl": offer_url }))
        .send()
        .await
        .expect("redeem request");

    let status = response.status();
    let body: Value = response.json().await.expect("redeem json");
    assert!(
        status.is_success(),
        "redeem at {holder} failed with {status}: {body:#}"
    );

    // Guards the two things that make the credential usable afterwards: it must be
    // recorded under the DCP credential type the verifier queries for, and under the
    // issuer's DID rather than the URL it was fetched from.
    assert_eq!(body["credentialType"], CREDENTIAL_TYPE);
    assert_eq!(body["issuer"], ISSUER_DID);
    assert_eq!(body["holder"], expected_holder_did);

    let held = get_json(client, &format!("{holder}/api/credentials/v1/credentials"))
        .await
        .expect("credential list");
    let matching = held
        .as_array()
        .expect("credential list is an array")
        .iter()
        .filter(|c| c["credentialType"] == CREDENTIAL_TYPE && c["issuer"] == ISSUER_DID)
        .count();
    assert_eq!(
        matching, 1,
        "{holder} should hold exactly one issuer-signed {CREDENTIAL_TYPE}, got: {held:#}"
    );
    println!("{holder} holds a {CREDENTIAL_TYPE} issued by {ISSUER_DID}");
}

/// Mint a Self-Issued ID Token at `requester` and exchange it at `peer`, which is the
/// full verifier path: SI token, presentation pull, VP and VC validation, claim mapping.
async fn exchange_token(client: &reqwest::Client, requester: &str, peer: &str, peer_did: &str) {
    let body = format!(
        "grant_type=client_credentials&client_id=dsp-client&client_secret=dsp-secret&audience={}",
        form_encode(peer_did)
    );
    let sts: Value = client
        .post(format!("{requester}/api-internal/sts/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .expect("sts request")
        .json()
        .await
        .expect("sts json");
    let si_token = sts["access_token"]
        .as_str()
        .unwrap_or_else(|| panic!("STS returned no access_token: {sts:#}"));

    let response = client
        .post(format!("{peer}/auth/token"))
        .bearer_auth(si_token)
        .send()
        .await
        .expect("auth/token request");

    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "{peer}/auth/token should succeed; a 401 means the SI token, the presentation \
         pull, the VP/VC validation or the claim mapping failed — check the peer's logs"
    );

    let token: Value = response.json().await.expect("auth/token json");
    assert_eq!(token["token_type"], "Bearer");
    let access_token = token["access_token"]
        .as_str()
        .unwrap_or_else(|| panic!("no access_token: {token:#}"));

    // `derive_access_token` returns None — surfacing as a 401 — unless the credential
    // subject carries both of these, so assert they actually landed on the token.
    let claims = jsonwebtoken::dangerous::insecure_decode::<Value>(access_token)
        .expect("access token decodes")
        .claims;
    assert_eq!(claims["email"], "johndoe@example.com");
    assert_eq!(claims["country"], "DE");
    assert_eq!(claims["sub"], requester_did(requester));
    println!("{requester} -> {peer}: 200, claims mapped (email, country)");
}

fn requester_did(base: &str) -> &'static str {
    match base {
        PARTY_A => A_DID,
        PARTY_B => B_DID,
        other => panic!("unknown party {other}"),
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires Docker; run: cargo test --test e2e_docker -- --ignored --nocapture"]
async fn e2e_waltid_issuance_then_dsp_transfer() {
    let _stack = Stack::up();
    let client = client();

    step!("waiting for connectors");
    for base in [PARTY_A, PARTY_B] {
        let did = wait_for(&format!("{base} DID document"), Duration::from_secs(120), || async {
            get_json(&client, &format!("{base}/.well-known/did.json")).await
        })
        .await;
        assert_eq!(did["id"], requester_did(base));
        // The verifier reaches the peer's wallet through this service entry.
        assert!(
            did["service"]
                .as_array()
                .is_some_and(|s| s.iter().any(|e| e["type"] == "CredentialService")),
            "{base} publishes no CredentialService: {did:#}"
        );
    }
    println!("both connectors serve their DID documents");

    step!("SETUP.md 2 — issuer metadata");
    assert_issuer_advertises_jwt_vc(&client).await;

    step!("SETUP.md 2 — seed credentials over OID4VCI");
    seed_credential(&client, PARTY_B, B_DID).await;
    seed_credential(&client, PARTY_A, A_DID).await;

    step!("SETUP.md 3 — DCP token exchange, both directions");
    exchange_token(&client, PARTY_B, PARTY_A, A_DID).await;
    exchange_token(&client, PARTY_A, PARTY_B, B_DID).await;

    // Catalog sync authenticates with the credential just seeded. The first sync ran at
    // startup and failed (nothing was seeded yet), which puts sync on its escalated
    // ~30s retry, so this resolves well inside the window. Until it lands, `negotiate`
    // has no federated dataset and answers 404.
    step!("USAGE.md — catalog sync and negotiation");
    let consumer_pid: String = wait_for(
        "party-b to discover party-a's dataset and start a negotiation",
        Duration::from_secs(180),
        || async {
            let response = client
                .post(format!("{PARTY_B}/api-internal/negotiate"))
                .json(&json!({
                    "name": "party-a",
                    "dataset_id": DATASET_ID,
                    "offer_id": OFFER_ID,
                }))
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            let body: Value = response.json().await.ok()?;
            body["consumer_pid"].as_str().map(str::to_string)
        },
    )
    .await;
    println!("negotiation started: {consumer_pid}");

    let agreement = wait_for("the negotiation to finalize", Duration::from_secs(90), || async {
        let state = get_json(
            &client,
            &format!("{PARTY_B}/api-internal/negotiate/{consumer_pid}"),
        )
        .await?;
        if state["state"]["type"] != "finalized" {
            return None;
        }
        Some(state["state"]["agreement"].clone())
    })
    .await;

    let agreement_id = agreement["@id"].as_str().expect("agreement @id").to_string();
    assert_eq!(agreement["target"], DATASET_ID);
    assert_eq!(agreement["assigner"], A_DID);
    assert_eq!(agreement["assignee"], B_DID);
    println!("negotiation finalized: {agreement_id}");

    step!("USAGE.md — transfer");
    let transfer: Value = client
        .post(format!("{PARTY_B}/api-internal/transfer"))
        .json(&json!({ "agreement_id": agreement_id, "format": "HttpData-PULL" }))
        .send()
        .await
        .expect("transfer request")
        .json()
        .await
        .expect("transfer json");
    let transfer_pid = transfer["consumer_pid"]
        .as_str()
        .unwrap_or_else(|| panic!("no transfer consumer_pid: {transfer:#}"))
        .to_string();

    let started = wait_for("the transfer to start", Duration::from_secs(90), || async {
        let state = get_json(
            &client,
            &format!("{PARTY_B}/api-internal/transfer/{transfer_pid}"),
        )
        .await?;
        (state["process"]["state"]["type"] == "started").then_some(state)
    })
    .await;
    println!("transfer started: {transfer_pid}");

    let data_address = &started["data_address"];
    assert_eq!(
        data_address["endpoint"], "http://party-a-connector:3000/pull",
        "unexpected data plane endpoint: {data_address:#}"
    );
    let pull_token = data_address["endpointProperties"]
        .as_array()
        .expect("endpointProperties")
        .iter()
        .find(|p| p["name"] == "authorization")
        .and_then(|p| p["value"].as_str())
        .unwrap_or_else(|| panic!("no authorization property: {data_address:#}"));

    step!("USAGE.md — pull the dataset");
    // The advertised endpoint is the in-network address; from the host it is the
    // published port, exactly as USAGE.md notes.
    let echo: Value = client
        .post(format!("{PARTY_A}/pull/test-path?test-query=true"))
        .bearer_auth(pull_token)
        // Sent as a raw body rather than via `.json()` so the bytes are exactly the
        // ones USAGE.md's `curl -d` sends, and the echoed base64 below can be compared
        // against the value that document records.
        .header("content-type", "application/json")
        .body(r#"{"sample-payload": 1}"#)
        .send()
        .await
        .expect("pull request")
        .json()
        .await
        .expect("pull json");

    assert_eq!(echo["method"], "POST");
    assert_eq!(echo["path"], "/api/test-path");
    assert_eq!(echo["query"]["test-query"], "true");
    // The echo target base64-encodes the body it received.
    assert_eq!(echo["body"], "eyJzYW1wbGUtcGF5bG9hZCI6IDF9");
    println!("dataset pulled through party-a's data plane");

    step!("end to end: walt.id issuance -> DCP -> negotiation -> transfer");
}
