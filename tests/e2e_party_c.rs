//! End-to-end interop test between this connector and a Java EDC connector.
//!
//! Party A is this connector; party C is the Eclipse EDC stack — control plane, data
//! plane and IdentityHub — built from the MinimumViableDataspace launchers. The test
//! drives a full DSP 2025-1 exchange over DCP in **both** directions, which is the point:
//! everything here has a second, independent implementation on the other side.
//!
//! Ignored by default — it needs Docker and party C's images. Run it with:
//!
//! ```sh
//! cd docker-compose && PARTY_C_ENABLED=true ./setup-configs.sh && ./build-party-c.sh
//! cargo test --test e2e_party_c -- --ignored --nocapture
//! ```
//!
//! One test function on purpose: the steps share one stack and one set of host ports.
//! For the same reason, do **not** run this together with `e2e_docker` — cargo runs test
//! binaries in parallel and the two would fight over the same containers and ports. It
//! tears the stack down and clears party A's credentials and synced catalog, so do not
//! run it against a stack you are using by hand.

use std::{path::Path, process::Command, time::Duration};

use serde_json::{Value, json};

const COMPOSE_FILE: &str = "docker-compose/docker-compose.yml";
const ISSUER: &str = "http://localhost:7002";

const PARTY_A: &str = "http://localhost:13000";
const A_DID: &str = "did:web:party-a-connector%3A3000";
/// Party A's DSP endpoint as party C must address it: complete, version path included.
const A_DSP: &str = "http://party-a-connector:3000/api/2025/1";
const A_DATASET: &str = "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57";

/// Party C's management API, its consumer-side data plane proxy, and its provider-side
/// public API, on the ports docker-compose publishes.
const C_MANAGEMENT: &str = "http://localhost:34081/api/mgmt";
const C_PROXY: &str = "http://localhost:34103/api/proxy";
const C_PUBLIC_HOST: &str = "http://localhost:34102";
const C_PUBLIC_INTERNAL: &str = "http://party-c-dataplane:11002";
const C_IDENTITY: &str = "http://localhost:34281/api/identity";
const C_HEALTH: &str = "http://localhost:34080/api/check/health";
const C_IH_HEALTH: &str = "http://localhost:34280/api/check/health";
const KEYCLOAK: &str = "http://localhost:34808";
const C_DID: &str = "did:web:party-c-identityhub%3A7083:party-c";
const C_ASSET: &str = "party-c-asset-1";
const C_MANAGEMENT_KEY: &str = "password";

const ISSUER_DID: &str = "did:web:issuer-did-server";
const PROFILE: &str = "identityCredentialJwtVc";
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
    let mut full = vec!["compose", "-f", COMPOSE_FILE, "--profile", "party-c"];
    full.extend_from_slice(args);
    docker(&full)
}

fn build_connector_image() {
    step!("building connector:latest");
    let out = docker(&["build", "-t", "connector:latest", "-f", "Dockerfile", "."]);
    assert!(
        out.status.success(),
        "docker build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Party C's images come from a Gradle build of a separate repository, so this test does
/// not build them — it fails early with the command that does.
fn assert_party_c_images_present() {
    for image in [
        "party-c-controlplane",
        "party-c-dataplane",
        "party-c-identityhub",
    ] {
        let out = docker(&["images", "-q", &format!("{image}:1.0.0-RC1")]);
        assert!(
            !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
            "{image}:1.0.0-RC1 is missing. Build party C's images first:\n  \
             cd docker-compose && ./build-party-c.sh"
        );
    }
}

/// The party C configs are rendered from templates, and only when party C is enabled.
fn assert_party_c_configured() {
    for file in [
        "docker-compose/party-c/controlplane/configuration.properties",
        "docker-compose/party-c/dataplane/configuration.properties",
        "docker-compose/party-c/identityhub/configuration.properties",
        "docker-compose/party-c/seed/seed.env",
    ] {
        assert!(
            Path::new(file).exists(),
            "{file} is missing. Generate party C's configuration first:\n  \
             cd docker-compose && PARTY_C_ENABLED=true ./setup-configs.sh"
        );
    }

    let config = std::fs::read_to_string("docker-compose/party-a/connector/config.json")
        .expect("party A's config should be generated; run ./setup-configs.sh");
    assert!(
        config.contains("party-c"),
        "party A does not federate party C. Set PARTY_C_ENABLED=true and re-run \
         ./setup-configs.sh"
    );
}

/// Start cold: no credentials and no synced catalog, so the test exercises issuance and
/// discovery rather than leftovers from an earlier run.
fn clear_party_a_state() {
    for dir in ["credentials", "datasets/federated"] {
        let path = format!("docker-compose/party-a/connector/data/{dir}");
        if Path::new(&path).exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

struct Stack;

impl Stack {
    fn up() -> Self {
        assert_party_c_configured();
        assert_party_c_images_present();
        build_connector_image();

        // `-v` so party C starts with an empty database and vault: its participant
        // context, keys and STS secret are provisioned by the seed step below, and a
        // half-seeded leftover is worse than nothing.
        let _ = compose(&["down", "-v"]);
        clear_party_a_state();

        step!("docker compose --profile party-c up");
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
        let _ = compose(&["down", "-v"]);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

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
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn get_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

/// Party C's management API is key-protected; every call here goes through this.
async fn management(client: &reqwest::Client, path: &str, body: Value) -> Value {
    let response = client
        .post(format!("{C_MANAGEMENT}{path}"))
        .header("x-api-key", C_MANAGEMENT_KEY)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("management POST {path} failed: {e}"));

    let status = response.status();
    let value: Value = response
        .json()
        .await
        .unwrap_or_else(|e| panic!("management POST {path} returned no json: {e}"));
    assert!(
        status.is_success(),
        "management POST {path} failed with {status}: {value:#}"
    );
    value
}

async fn management_get(client: &reqwest::Client, path: &str) -> Option<Value> {
    let response = client
        .get(format!("{C_MANAGEMENT}{path}"))
        .header("x-api-key", C_MANAGEMENT_KEY)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

/// Party C's Identity API is OAuth2-protected, because the IdentityHub launcher MVD
/// builds is the OAuth2 variant.
async fn keycloak_token(client: &reqwest::Client) -> String {
    let token = wait_for("a keycloak token", Duration::from_secs(180), || async {
        let response = client
            .post(format!(
                "{KEYCLOAK}/realms/mvd/protocol/openid-connect/token"
            ))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(
                "grant_type=client_credentials&client_id=admin\
                 &client_secret=edc-v-admin-secret&scope=identity-api%3Aadmin",
            )
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: Value = response.json().await.ok()?;
        body["access_token"].as_str().map(str::to_string)
    })
    .await;
    token
}

/// A catalog carries one dataset as an object and several as an array, depending on how
/// the peer compacted the JSON-LD. Accept either.
fn dataset_by_id(catalog: &Value, id: &str) -> Option<Value> {
    let datasets = &catalog["dataset"];
    if let Some(entries) = datasets.as_array() {
        entries.iter().find(|d| d["@id"] == id).cloned()
    } else if datasets["@id"] == id {
        Some(datasets.clone())
    } else {
        None
    }
}

fn find_property<'a>(data_address: &'a Value, name: &str) -> Option<&'a str> {
    data_address["endpointProperties"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == name)?["value"]
        .as_str()
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

/// Ask the bundled issuer for a pre-authorized offer and have party A redeem it. Party C
/// gets its credential a different way — see `seed_party_c`.
async fn seed_party_a_credential(client: &reqwest::Client) {
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

    let body: Value = client
        .post(format!("{PARTY_A}/api-internal/credentials/redeem"))
        .json(&json!({ "offerUrl": offer_url }))
        .send()
        .await
        .expect("redeem request")
        .json()
        .await
        .expect("redeem json");

    assert_eq!(body["credentialType"], CREDENTIAL_TYPE);
    assert_eq!(body["issuer"], ISSUER_DID);
    assert_eq!(body["holder"], A_DID);
    println!("party A holds a {CREDENTIAL_TYPE} issued by {ISSUER_DID}");
}

/// Runs the seed container, which creates party C's participant context, requests an
/// identity credential from **party A's** DCP issuer, registers party C's data plane and
/// publishes one asset for party A to consume.
fn seed_party_c() {
    // The seed sits in its own compose profile so that `up` does not run it — see the
    // comment on the service. Naming it enables that profile; `--profile party-c` is
    // still needed for the services it depends on.
    let out = docker(&[
        "compose",
        "-f",
        COMPOSE_FILE,
        "--profile",
        "party-c",
        "--profile",
        "seed",
        "run",
        "--rm",
        "party-c-seed",
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "party C seeding failed:\n{stdout}\n{stderr}"
    );
    println!("{}", stdout.trim());
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires Docker and party C's images; run: cargo test --test e2e_party_c -- --ignored --nocapture"]
async fn e2e_interop_with_a_java_edc_connector() {
    let _stack = Stack::up();
    let client = client();

    step!("waiting for both stacks");
    let did = wait_for(
        "party A's DID document",
        Duration::from_secs(180),
        || async { get_json(&client, &format!("{PARTY_A}/.well-known/did.json")).await },
    )
    .await;
    assert_eq!(did["id"], A_DID);

    wait_for(
        "party C's control plane",
        Duration::from_secs(300),
        || async { get_json(&client, C_HEALTH).await },
    )
    .await;
    wait_for(
        "party C's IdentityHub",
        Duration::from_secs(300),
        || async { get_json(&client, C_IH_HEALTH).await },
    )
    .await;
    println!("party A and party C are up");

    step!("the API paths party C actually serves");
    // Read these off the running jars rather than trusting documentation: the management
    // API and the Identity API have both moved between EDC versions, and a wrong guess
    // shows up as a 404 much later.
    let versions = wait_for(
        "party C's API manifest",
        Duration::from_secs(120),
        || async { get_json(&client, "http://localhost:34080/api/v1/version").await },
    )
    .await;
    let management_paths: Vec<&str> = versions["management"]
        .as_array()
        .expect("a management API entry")
        .iter()
        .filter_map(|v| v["urlPath"].as_str())
        .collect();
    assert!(
        management_paths.contains(&"/v4"),
        "party C no longer serves the management API at /v4, this test uses it: {management_paths:?}"
    );
    let identity_versions = wait_for(
        "party C's IdentityHub manifest",
        Duration::from_secs(120),
        || async { get_json(&client, "http://localhost:34280/api/v1/version").await },
    )
    .await;
    assert_eq!(
        identity_versions["identity"][0]["urlPath"], "/v1",
        "the Identity API moved; the seed script targets /v1"
    );
    println!("management API at /v4, Identity API at /v1");

    step!("credentials — party A from walt.id, party C from party A's DCP issuer");
    seed_party_a_credential(&client).await;
    seed_party_c();

    let identity_token = keycloak_token(&client).await;
    let held = wait_for(
        "party C to hold an identity credential",
        Duration::from_secs(120),
        || async {
            let response = client
                .get(format!("{C_IDENTITY}/v1/participants/party-c/credentials"))
                .bearer_auth(&identity_token)
                .send()
                .await
                .ok()?;
            let credentials: Value = response.json().await.ok()?;
            let issued_by_a = credentials.as_array()?.iter().any(|c| {
                c["verifiableCredential"]["credential"]["issuer"]["id"] == A_DID
                    && c["verifiableCredential"]["credential"]["type"]
                        .as_array()
                        .is_some_and(|t| t.iter().any(|t| t == CREDENTIAL_TYPE))
            });
            issued_by_a.then_some(credentials)
        },
    )
    .await;
    println!(
        "party C holds {} credential(s), issued by party A over DCP",
        held.as_array().map(Vec::len).unwrap_or(0)
    );

    // -----------------------------------------------------------------------
    // Party C consumes from party A
    // -----------------------------------------------------------------------

    step!("C -> A — catalog");
    // Party C authenticates with a self-issued token on the request itself; party A
    // verifies it by pulling a presentation from party C's IdentityHub.
    let dataset = wait_for("party A's catalog", Duration::from_secs(120), || async {
        let catalog = management(
            &client,
            "/v4/catalog/request",
            json!({
                "@context": ["https://w3id.org/edc/connector/management/v2"],
                "@type": "CatalogRequest",
                "counterPartyAddress": A_DSP,
                "counterPartyId": A_DID,
                "protocol": "dataspace-protocol-http:2025-1"
            }),
        )
        .await;
        dataset_by_id(&catalog, A_DATASET)
    })
    .await;

    let offer = dataset["hasPolicy"][0].clone();
    assert!(
        offer["@id"].is_string(),
        "party A's dataset carries no offer: {dataset:#}"
    );
    let offer_id = offer["@id"].as_str().expect("an offer id").to_string();
    println!("party C read party A's catalog: {A_DATASET}, offer {offer_id}");

    step!("C -> A — contract negotiation");
    let mut requested_offer = offer;
    requested_offer["assigner"] = json!(A_DID);
    requested_offer["target"] = json!(A_DATASET);
    let negotiation = management(
        &client,
        "/v4/contractnegotiations",
        json!({
            "@context": ["https://w3id.org/edc/connector/management/v2"],
            "@type": "ContractRequest",
            "counterPartyAddress": A_DSP,
            "counterPartyId": A_DID,
            "protocol": "dataspace-protocol-http:2025-1",
            "policy": requested_offer
        }),
    )
    .await;
    let negotiation_id = negotiation["@id"]
        .as_str()
        .expect("a negotiation id")
        .to_string();

    let agreement_id = wait_for(
        "party C's negotiation to finalize",
        Duration::from_secs(120),
        || async {
            let state = management_get(
                &client,
                &format!("/v4/contractnegotiations/{negotiation_id}"),
            )
            .await?;
            assert_ne!(
                state["state"], "TERMINATED",
                "negotiation terminated: {state:#}"
            );
            (state["state"] == "FINALIZED")
                .then(|| state["contractAgreementId"].as_str().map(str::to_string))
                .flatten()
        },
    )
    .await;
    println!("agreement: {agreement_id}");

    step!("C -> A — transfer");
    let transfer = management(
        &client,
        "/v4/transferprocesses",
        json!({
            "@context": ["https://w3id.org/edc/connector/management/v2"],
            "@type": "TransferRequest",
            "assetId": A_DATASET,
            "counterPartyAddress": A_DSP,
            "connectorId": A_DID,
            "contractId": agreement_id,
            "dataDestination": { "@type": "DataAddress", "type": "HttpProxy" },
            "protocol": "dataspace-protocol-http:2025-1",
            "transferType": "HttpData-PULL"
        }),
    )
    .await;
    let transfer_id = transfer["@id"].as_str().expect("a transfer id").to_string();

    wait_for(
        "party C's transfer to start",
        Duration::from_secs(120),
        || async {
            let state =
                management_get(&client, &format!("/v4/transferprocesses/{transfer_id}")).await?;
            assert_ne!(
                state["state"], "TERMINATED",
                "transfer terminated: {state:#}"
            );
            (state["state"] == "STARTED").then_some(())
        },
    )
    .await;
    println!("transfer started: {transfer_id}");

    step!("C -> A — pull the dataset");
    let flow = wait_for(
        "party C's data plane to record the flow",
        Duration::from_secs(60),
        || async { get_json(&client, &format!("{C_PROXY}/flows/{transfer_id}")).await },
    )
    .await;

    let pull_token = find_property(&flow, "authorization")
        .unwrap_or_else(|| panic!("party A sent no authorization property: {flow:#}"));
    // Party A sends the endpoint as `dspace:endpoint`, which is what DSP defines, but the
    // Java EDC reads it from its own namespace and so drops it. The token survives, so
    // the pull below addresses party A directly. Assert the gap rather than working
    // around it silently — if EDC starts reading `dspace:endpoint`, this fires and the
    // endpoint can be taken from the flow.
    assert!(
        flow["endpoint"].is_null(),
        "party C now reads dspace:endpoint ({}); use it instead of addressing party A directly",
        flow["endpoint"]
    );

    let echo: Value = client
        .post(format!("{PARTY_A}/pull/test-path?test-query=true"))
        .bearer_auth(pull_token)
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
    assert_eq!(echo["body"], "eyJzYW1wbGUtcGF5bG9hZCI6IDF9");
    println!("party C pulled party A's dataset");

    // -----------------------------------------------------------------------
    // Party A consumes from party C
    // -----------------------------------------------------------------------

    step!("A -> C — catalog discovery");
    // Party C's asset only exists after the seed above, and party A syncs a peer's
    // catalog on a 10-minute interval. Restart it so the sync happens now rather than
    // making the test wait out the interval.
    let out = compose(&["restart", "party-a-connector"]);
    assert!(
        out.status.success(),
        "restarting party A failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    wait_for("party A to come back", Duration::from_secs(120), || async {
        get_json(&client, &format!("{PARTY_A}/.well-known/did.json")).await
    })
    .await;

    // Party A discovers party C's DSP endpoint from the DataService entry in party C's
    // DID document, then syncs its catalog.
    let federated = wait_for(
        "party A to sync party C's catalog",
        Duration::from_secs(240),
        || async {
            let path = format!(
                "docker-compose/party-a/connector/data/datasets/federated/party-c/{C_ASSET}.json"
            );
            let raw = std::fs::read_to_string(path).ok()?;
            serde_json::from_str::<Value>(&raw).ok()
        },
    )
    .await;
    let c_dataset = federated.get("dataset").unwrap_or(&federated);
    let c_offer_id = c_dataset["hasPolicy"][0]["@id"]
        .as_str()
        .expect("an offer id on party C's dataset")
        .to_string();
    println!("party A discovered {C_ASSET}, offer {c_offer_id}");

    step!("A -> C — contract negotiation");
    let negotiation: Value = client
        .post(format!("{PARTY_A}/api-internal/negotiate"))
        .json(&json!({
            "name": "party-c",
            "dataset_id": C_ASSET,
            "offer_id": c_offer_id
        }))
        .send()
        .await
        .expect("negotiate request")
        .json()
        .await
        .expect("negotiate json");
    let consumer_pid = negotiation["consumer_pid"]
        .as_str()
        .unwrap_or_else(|| panic!("no consumer_pid: {negotiation:#}"))
        .to_string();

    let agreement = wait_for(
        "party A's negotiation to finalize",
        Duration::from_secs(120),
        || async {
            let state = get_json(
                &client,
                &format!("{PARTY_A}/api-internal/negotiate/{consumer_pid}"),
            )
            .await?;
            assert_ne!(
                state["state"]["type"], "terminated",
                "negotiation terminated: {state:#}"
            );
            (state["state"]["type"] == "finalized").then(|| state["state"]["agreement"].clone())
        },
    )
    .await;
    let c_agreement_id = agreement["@id"]
        .as_str()
        .expect("an agreement id")
        .to_string();
    assert_eq!(agreement["assigner"], C_DID);
    assert_eq!(agreement["assignee"], A_DID);
    println!("agreement: {c_agreement_id}");

    step!("A -> C — transfer");
    let transfer: Value = client
        .post(format!("{PARTY_A}/api-internal/transfer"))
        .json(&json!({ "agreement_id": c_agreement_id, "format": "HttpData-PULL" }))
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

    let started = wait_for(
        "party A's transfer to start",
        Duration::from_secs(120),
        || async {
            let state = get_json(
                &client,
                &format!("{PARTY_A}/api-internal/transfer/{transfer_pid}"),
            )
            .await?;
            (state["process"]["state"]["type"] == "started").then_some(state)
        },
    )
    .await;
    println!("transfer started: {transfer_pid}");

    step!("A -> C — pull the dataset");
    let data_address = &started["data_address"];
    let endpoint = data_address["endpoint"]
        .as_str()
        .unwrap_or_else(|| panic!("party C sent no endpoint: {data_address:#}"));
    let access_token = find_property(data_address, "access_token")
        .unwrap_or_else(|| panic!("party C sent no access_token: {data_address:#}"));
    assert!(
        endpoint.starts_with(C_PUBLIC_INTERNAL),
        "unexpected data plane endpoint: {endpoint}"
    );

    // The advertised endpoint is party C's in-network address; from the host it is the
    // published port.
    let host_endpoint = endpoint.replace(C_PUBLIC_INTERNAL, C_PUBLIC_HOST);
    let response = client
        .get(&host_endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .expect("pull request");
    assert!(
        response.status().is_success(),
        "pull from party C failed with {}",
        response.status()
    );
    let payload = response.text().await.expect("pull body");
    let rows: Value = serde_json::from_str(&payload)
        .unwrap_or_else(|e| panic!("party C's data plane returned non-json ({e}): {payload}"));
    assert!(
        rows.as_array().is_some_and(|r| !r.is_empty()),
        "party C's data plane returned no rows: {payload}"
    );
    println!(
        "party A pulled party C's data ({} rows)",
        rows.as_array().unwrap().len()
    );

    step!("interop verified in both directions");
}
