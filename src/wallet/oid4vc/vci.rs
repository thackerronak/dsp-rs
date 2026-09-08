//! OID4VCI — redeeming a credential offer from an external issuer.
//!
//! The connector is the holder here. It runs the **pre-authorized code** flow against
//! an OpenID4VCI issuer (walt.id in the demo), proves possession of its own key, and
//! stores the resulting Verifiable Credential.
//!
//! This is provisioning, not protocol: it runs once to seed a credential, after which
//! DCP handles presentation. It exists because the connector holds its own key — the
//! alternative would be handing the private key to an external wallet to redeem on its
//! behalf.

use std::{collections::HashMap, sync::Arc};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::wallet::{
    KeyPair,
    store::{CredentialStore, StoredCredential},
};

const PRE_AUTHORIZED_GRANT: &str = "urn:ietf:params:oauth:grant-type:pre-authorized_code";
const PROOF_TYP: &str = "openid4vci-proof+jwt";

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CredentialOffer {
    credential_issuer: String,
    #[serde(default)]
    credential_configuration_ids: Vec<String>,
    #[serde(default)]
    grants: Grants,
}

#[derive(Debug, Default, Deserialize)]
struct Grants {
    #[serde(rename = "urn:ietf:params:oauth:grant-type:pre-authorized_code")]
    pre_authorized: Option<PreAuthorizedGrant>,
}

#[derive(Debug, Deserialize)]
struct PreAuthorizedGrant {
    #[serde(rename = "pre-authorized_code")]
    pre_authorized_code: String,
}

#[derive(Debug, Deserialize)]
struct IssuerMetadata {
    credential_endpoint: String,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    token_endpoint: Option<String>,
    /// Newer OID4VCI drafts moved nonce issuance out of the token response and onto a
    /// dedicated endpoint, which the issuer advertises here.
    #[serde(default)]
    nonce_endpoint: Option<String>,
    #[serde(default)]
    credential_configurations_supported: HashMap<String, CredentialConfiguration>,
    /// Whether the issuer takes a pre-authorized code with no `client_id` at all.
    #[serde(default, rename = "pre-authorized_grant_anonymous_access_supported")]
    anonymous_pre_authorized_access: bool,
}

#[derive(Debug, Deserialize)]
struct CredentialConfiguration {
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthServerMetadata {
    token_endpoint: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    c_nonce: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NonceResponse {
    c_nonce: String,
}

#[derive(Debug, Serialize)]
struct ProofClaims {
    aud: String,
    iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
}

/// The credential endpoint has varied across OID4VCI drafts: a bare `credential`
/// string, or a `credentials` array of either strings or `{ credential: "..." }`.
#[derive(Debug, Deserialize)]
struct CredentialResponse {
    #[serde(default)]
    credential: Option<Value>,
    #[serde(default)]
    credentials: Option<Vec<Value>>,
}

impl CredentialResponse {
    fn into_jwt(self) -> anyhow::Result<String> {
        let candidate = self
            .credential
            .or_else(|| self.credentials.and_then(|list| list.into_iter().next()))
            .ok_or_else(|| anyhow::anyhow!("credential response contained no credential"))?;

        match candidate {
            Value::String(jwt) => Ok(jwt),
            Value::Object(ref map) => map
                .get("credential")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("credential object has no `credential` field")),
            other => anyhow::bail!("unexpected credential shape: {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Oid4vciState {
    inner: Arc<Oid4vciInner>,
}

struct Oid4vciInner {
    key_pair: KeyPair,
    holder_did: String,
    kid: String,
    client: Client,
    store: Arc<dyn CredentialStore>,
    /// Configured issuer base URL. An offer naming a different issuer is refused, so a
    /// stray offer URL cannot make the connector redeem from somewhere arbitrary.
    issuer_url: String,
}

impl Oid4vciState {
    pub(crate) fn new(
        key_pair: KeyPair,
        holder_did: String,
        kid: String,
        client: Client,
        store: Arc<dyn CredentialStore>,
        issuer_url: String,
    ) -> Self {
        Self {
            inner: Arc::new(Oid4vciInner {
                key_pair,
                holder_did,
                kid,
                client,
                store,
                issuer_url,
            }),
        }
    }

    /// Run the pre-authorized code flow and store the credential.
    pub(crate) async fn redeem(&self, offer_input: &str) -> anyhow::Result<StoredCredential> {
        let inner = &self.inner;

        let offer = parse_offer(&inner.client, offer_input).await?;
        let issuer = offer.credential_issuer.trim_end_matches('/').to_string();

        if !inner.issuer_url.is_empty() {
            let configured = inner.issuer_url.trim_end_matches('/');
            anyhow::ensure!(
                same_origin(configured, &issuer),
                "offer names issuer `{issuer}`, which is not the configured issuer `{configured}`"
            );
        }

        let grant = offer
            .grants
            .pre_authorized
            .ok_or_else(|| anyhow::anyhow!("offer carries no pre-authorized code grant"))?;

        let configuration_id = offer
            .credential_configuration_ids
            .first()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("offer names no credential configuration"))?;

        let metadata = fetch_issuer_metadata(&inner.client, &issuer).await?;
        let token_endpoint = resolve_token_endpoint(&inner.client, &issuer, &metadata).await?;

        // `client_id` is required unless the issuer advertises
        // `pre-authorized_grant_anonymous_access_supported`. walt.id does not set it and
        // rejects an anonymous request with `invalid_client`, so identify the holder by
        // its DID. An issuer that does advertise it may have no notion of this client at
        // all — Veres answers a request carrying one with a 500 — so send nothing.
        let mut body = format!(
            "grant_type={}&pre-authorized_code={}",
            form_encode(PRE_AUTHORIZED_GRANT),
            form_encode(&grant.pre_authorized_code),
        );
        if !metadata.anonymous_pre_authorized_access {
            body.push_str(&format!("&client_id={}", form_encode(&inner.holder_did)));
        }

        let token: TokenResponse = json_or_error(
            inner
                .client
                .post(&token_endpoint)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await?,
            "token request",
        )
        .await?;

        // Older issuers return `c_nonce` on the token response; newer ones expect the
        // holder to fetch one from the nonce endpoint instead.
        let nonce = match token.c_nonce.clone() {
            Some(nonce) => Some(nonce),
            None => fetch_nonce(&inner.client, &metadata).await,
        };

        let proof = inner.key_pair.encode_with_header(
            ProofClaims {
                aud: issuer.clone(),
                iat: chrono::Utc::now().timestamp(),
                nonce,
            },
            inner.kid.clone(),
            PROOF_TYP,
        )?;

        let configuration = metadata
            .credential_configurations_supported
            .get(&configuration_id);

        let response: CredentialResponse = json_or_error(
            inner
                .client
                .post(&metadata.credential_endpoint)
                .bearer_auth(&token.access_token)
                .json(&json!({
                    "credential_configuration_id": configuration_id,
                    // OID4VCI draft 15 replaced the single `proof` object with `proofs`,
                    // keyed by proof type. walt.id reads only `proofs` and answers
                    // `invalid_proof: Credential request is missing proofs` without it,
                    // while older issuers read only `proof` — so send both.
                    "proof": { "proof_type": "jwt", "jwt": &proof },
                    "proofs": { "jwt": [&proof] }
                }))
                .send()
                .await?,
            "credential request",
        )
        .await?;

        let jwt = response.into_jwt()?;

        // A DCP presentation query names a credential *type*: the verifier asks for
        // `org.eclipse.dspace.dcp.vc.type:identity_credential` and the holder matches
        // `credential_type` exactly. The OID4VCI configuration id
        // (`IdentityCredential_jwt_vc_json`) is a deployment label for the format, not
        // that type — the value both sides agree on is the configuration's advertised
        // `scope`. Storing the configuration id left the credential unmatchable, so the
        // peer returned an empty presentation list.
        let credential_type = configuration
            .and_then(|config| config.scope.clone())
            .or_else(|| credential_type_from_vc(&jwt))
            .unwrap_or_else(|| configuration_id.clone());

        let stored = StoredCredential {
            id: format!("{issuer}#{configuration_id}"),
            credential_type,
            format: "vc+jwt".to_string(),
            issuer: credential_issuer_did(&jwt).unwrap_or_else(|| issuer.clone()),
            payload: jwt,
        };

        inner.store.store(&stored).await?;

        Ok(stored)
    }

    pub(crate) fn holder_did(&self) -> &str {
        &self.inner.holder_did
    }
}

/// Reads a received JWT's claims without verifying anything — the signature is the
/// verifier's job, later, once the credential is presented.
///
/// Decoded by hand rather than through `jsonwebtoken`, whose `Algorithm` enum has no
/// ES256K. Issuers do sign with it — walt.id's public demo among them — and going
/// through the library turned a perfectly readable claim set into `None`.
fn jwt_claims(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

/// The VC's `iss` is the issuer's DID, which is what `allowed_issuers` is matched
/// against — not the HTTPS base URL the credential was fetched from.
fn credential_issuer_did(jwt: &str) -> Option<String> {
    jwt_claims(jwt)?.get("iss")?.as_str().map(str::to_string)
}

/// The DCP type a verifier queries for, read off the credential itself.
///
/// Preferred source is the configuration's advertised `scope`, but not every issuer
/// publishes one. The credential's own `vc.type` carries the same information in the
/// other spelling — the bundled issuer's `scope` is exactly the snake_case of its most
/// specific type — so derive it rather than storing a label no verifier will match.
fn credential_type_from_vc(jwt: &str) -> Option<String> {
    const BASE_TYPE: &str = "VerifiableCredential";

    let specific = jwt_claims(jwt)?
        .get("vc")?
        .get("type")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter(|ty| *ty != BASE_TYPE)
        .next_back()?
        .to_string();

    Some(to_snake_case(&specific))
}

fn to_snake_case(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 4);

    for (index, ch) in value.char_indices() {
        if ch.is_uppercase() && index != 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }

    out
}

/// `application/x-www-form-urlencoded`, leaving the RFC 3986 unreserved characters
/// alone. `NON_ALPHANUMERIC` would also escape `-._~`, which is legal but unusual
/// enough that a strict parser on the far side may balk.
const FORM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn form_encode(value: &str) -> String {
    utf8_percent_encode(value, FORM).to_string()
}

fn same_origin(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(a), Ok(b)) => a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default(),
        _ => a == b,
    }
}

/// Accepts `openid-credential-offer://?credential_offer=<json>`, the
/// `credential_offer_uri=<url>` by-reference form, or raw offer JSON.
async fn parse_offer(client: &Client, input: &str) -> anyhow::Result<CredentialOffer> {
    let trimmed = input.trim();

    if trimmed.starts_with('{') {
        return Ok(serde_json::from_str(trimmed)?);
    }

    let url = Url::parse(trimmed).map_err(|err| anyhow::anyhow!("invalid offer URL: {err}"))?;

    // Decoded from the raw query rather than via `query_pairs()`: that applies
    // form-decoding, which turns `+` into a space and would corrupt JSON payloads.
    if let Some(raw) = raw_query_value(&url, "credential_offer") {
        let decoded = percent_decode_str(raw).decode_utf8()?;
        return Ok(serde_json::from_str(&decoded)?);
    }

    if let Some(raw) = raw_query_value(&url, "credential_offer_uri") {
        let uri = percent_decode_str(raw).decode_utf8()?;
        return Ok(client
            .get(uri.as_ref())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?);
    }

    anyhow::bail!("offer URL has neither `credential_offer` nor `credential_offer_uri`")
}

fn raw_query_value<'u>(url: &'u Url, key: &str) -> Option<&'u str> {
    url.query()?
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
}

/// `error_for_status` throws the response body away, and that body is exactly where
/// OAuth and OID4VCI put the reason a request was refused. Keeping it turns an opaque
/// "400 Bad Request" into a diagnosable error.
async fn json_or_error<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    what: &str,
) -> anyhow::Result<T> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    anyhow::ensure!(status.is_success(), "{what} failed with {status}: {body}");

    serde_json::from_str(&body)
        .map_err(|err| anyhow::anyhow!("{what} returned an unreadable body ({err}): {body}"))
}

/// An issuer identifier may carry a path — walt.id publishes
/// `http://host:7002/openid4vci`. RFC 8615 puts the well-known segment immediately after
/// the authority and appends the issuer's own path *after* it, so the metadata for
/// `http://host/openid4vci` is at
/// `http://host/.well-known/openid-credential-issuer/openid4vci` and **not** at
/// `http://host/openid4vci/.well-known/openid-credential-issuer`. Naive concatenation
/// 404s against any issuer whose identifier is not bare-origin.
///
/// Deployments disagree, though: walt.id's public demo serves only the concatenated
/// form. Both are returned, spec placement first, so discovery can fall back.
fn well_known_urls(issuer: &str, suffix: &str) -> Vec<String> {
    let Ok(url) = Url::parse(issuer) else {
        return vec![format!(
            "{}/.well-known/{suffix}",
            issuer.trim_end_matches('/')
        )];
    };

    let path = url.path().trim_matches('/').to_string();
    let mut origin = url;
    origin.set_path("");
    origin.set_query(None);
    origin.set_fragment(None);
    let origin = origin.as_str().trim_end_matches('/');

    if path.is_empty() {
        vec![format!("{origin}/.well-known/{suffix}")]
    } else {
        vec![
            format!("{origin}/.well-known/{suffix}/{path}"),
            format!("{origin}/{path}/.well-known/{suffix}"),
        ]
    }
}

async fn fetch_issuer_metadata(client: &Client, issuer: &str) -> anyhow::Result<IssuerMetadata> {
    let mut last_error = None;

    for url in well_known_urls(issuer, "openid-credential-issuer") {
        let attempt = match client.get(&url).send().await {
            Ok(response) => json_or_error(response, "issuer metadata").await,
            Err(err) => Err(anyhow::Error::new(err)),
        };

        match attempt {
            Ok(metadata) => return Ok(metadata),
            Err(err) => last_error = Some(err.context(format!("issuer metadata at {url}"))),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("issuer `{issuer}` yields no metadata URL")))
}

/// Best effort: an issuer that neither returns `c_nonce` nor exposes a nonce endpoint
/// just gets an unbound proof, which it is free to reject.
async fn fetch_nonce(client: &Client, metadata: &IssuerMetadata) -> Option<String> {
    let endpoint = metadata.nonce_endpoint.as_deref()?;

    match client.post(endpoint).send().await {
        Ok(response) => match response.error_for_status() {
            Ok(response) => match response.json::<NonceResponse>().await {
                Ok(nonce) => Some(nonce.c_nonce),
                Err(err) => {
                    tracing::warn!("nonce endpoint returned an unreadable body: {err}");
                    None
                }
            },
            Err(err) => {
                tracing::warn!("nonce endpoint rejected the request: {err}");
                None
            }
        },
        Err(err) => {
            tracing::warn!("nonce endpoint unreachable: {err}");
            None
        }
    }
}

/// Per OID4VCI the token endpoint lives on the authorization server, which may be the
/// issuer itself. Some deployments publish it inline instead.
async fn resolve_token_endpoint(
    client: &Client,
    issuer: &str,
    metadata: &IssuerMetadata,
) -> anyhow::Result<String> {
    if let Some(endpoint) = &metadata.token_endpoint {
        return Ok(endpoint.clone());
    }

    let auth_server = metadata
        .authorization_servers
        .first()
        .map(String::as_str)
        .unwrap_or(issuer)
        .trim_end_matches('/');

    for discovery in well_known_urls(auth_server, "oauth-authorization-server") {
        if let Ok(response) = client.get(&discovery).send().await
            && let Ok(response) = response.error_for_status()
            && let Ok(metadata) = response.json::<AuthServerMetadata>().await
        {
            return Ok(metadata.token_endpoint);
        }
    }

    Ok(format!("{auth_server}/token"))
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RedeemRequest {
    /// `offerUrl` is what SETUP.md documents and matches the camelCase used across the
    /// DCP wire types; it was previously rejected with a 422 because only the
    /// snake_case and bare `offer` spellings were accepted.
    #[serde(rename = "offerUrl", alias = "offer_url", alias = "offer")]
    offer_url: String,
}

pub(crate) fn router(state: Oid4vciState) -> Router {
    Router::new()
        .route("/redeem", post(redeem))
        .with_state(state)
}

async fn redeem(
    State(state): State<Oid4vciState>,
    Json(request): Json<RedeemRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let stored = state.redeem(&request.offer_url).await.map_err(|err| {
        tracing::warn!("OID4VCI redemption failed: {err}");
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": err.to_string() })),
        )
    })?;

    Ok(Json(json!({
        "id": stored.id,
        "credentialType": stored.credential_type,
        "issuer": stored.issuer,
        "holder": state.holder_did(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use axum::{extract::State as AxumState, routing::get};
    use jsonwebtoken::decode_header;

    use crate::wallet::{
        dcp::issuer::mint_identity_credential,
        store::FileCredentialStore,
        test_support::{HOLDER_DID, ISSUER_DID, TestServer, kid, test_key_pair},
    };

    const PRE_AUTH_CODE: &str = "pre-auth-code-123";
    const NONCE: &str = "nonce-abc";
    const CONFIG_ID: &str = "IdentityCredential_jwt_vc_json";
    /// The DCP credential type the verifier's scope query names, advertised by the issuer
    /// as the credential configuration's `scope`.
    const CREDENTIAL_SCOPE: &str = "identity_credential";
    /// walt.id's issuer identifier is not bare-origin.
    const ISSUER_PATH: &str = "/openid4vci";

    #[derive(Clone)]
    struct MockIssuer {
        vc: String,
        seen_proof: Arc<Mutex<Option<String>>>,
    }

    /// Shaped like walt.id: the issuer identifier carries a `/openid4vci` path, so its
    /// metadata lives at `/.well-known/openid-credential-issuer/openid4vci`; the token
    /// endpoint is only discoverable through authorization-server metadata; the token
    /// response carries no `c_nonce`; and anonymous pre-authorized access is refused.
    fn mock_issuer_router(state: MockIssuer) -> Router {
        // Self-URLs come from the Host header: the server's port is only known once it is
        // bound, which is after this router is built.
        fn base(headers: &axum::http::HeaderMap) -> String {
            let host = headers
                .get("host")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("127.0.0.1");
            format!("http://{host}")
        }

        Router::new()
            .route(
                "/.well-known/openid-credential-issuer/openid4vci",
                get(|headers: axum::http::HeaderMap| async move {
                    let base = base(&headers);
                    Json(json!({
                        "credential_issuer": format!("{base}{ISSUER_PATH}"),
                        "credential_endpoint": format!("{base}{ISSUER_PATH}/credential"),
                        "nonce_endpoint": format!("{base}{ISSUER_PATH}/nonce"),
                        "credential_configurations_supported": {
                            CONFIG_ID: { "format": "jwt_vc_json", "scope": CREDENTIAL_SCOPE }
                        }
                    }))
                }),
            )
            .route(
                "/.well-known/oauth-authorization-server/openid4vci",
                get(|headers: axum::http::HeaderMap| async move {
                    let base = base(&headers);
                    Json(json!({ "token_endpoint": format!("{base}{ISSUER_PATH}/token") }))
                }),
            )
            .route(
                "/openid4vci/token",
                post(|body: String| async move {
                    assert!(
                        body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Apre-authorized_code"),
                        "unexpected grant_type in {body}"
                    );
                    assert!(body.contains(PRE_AUTH_CODE), "missing code in {body}");
                    assert!(
                        body.contains("client_id="),
                        "anonymous pre-authorized access is refused; expected client_id in {body}"
                    );
                    // Deliberately no `c_nonce` — the holder must use the nonce endpoint.
                    Json(json!({ "access_token": "issuer-access-token" }))
                }),
            )
            .route(
                "/openid4vci/nonce",
                post(|| async { Json(json!({ "c_nonce": NONCE })) }),
            )
            .route(
                "/openid4vci/credential",
                post(
                    |AxumState(s): AxumState<MockIssuer>,
                     headers: axum::http::HeaderMap,
                     Json(body): Json<Value>| async move {
                        assert_eq!(
                            headers.get("authorization").unwrap(),
                            "Bearer issuer-access-token"
                        );
                        // Draft 15 shape: `proofs` keyed by proof type.
                        let jwt = body["proofs"]["jwt"][0]
                            .as_str()
                            .expect("proofs.jwt[0]")
                            .to_string();
                        assert_eq!(
                            body["proof"]["jwt"].as_str(),
                            Some(jwt.as_str()),
                            "the legacy `proof` form should carry the same JWT"
                        );
                        *s.seen_proof.lock().unwrap() = Some(jwt);
                        Json(json!({ "credential": s.vc }))
                    },
                ),
            )
            .with_state(state)
    }

    async fn spawn_issuer() -> (TestServer, Arc<Mutex<Option<String>>>) {
        let key_pair = test_key_pair();
        let vc = mint_identity_credential(
            &key_pair,
            ISSUER_DID,
            &kid(ISSUER_DID),
            HOLDER_DID,
            json!({ "email": "ops@party.example", "address": { "country": "DE" } }),
        )
        .unwrap();

        let seen_proof = Arc::new(Mutex::new(None));
        let state = MockIssuer {
            vc,
            seen_proof: seen_proof.clone(),
        };
        let server = TestServer::spawn(mock_issuer_router(state)).await;
        (server, seen_proof)
    }

    fn wallet(store: Arc<dyn CredentialStore>, issuer_url: String) -> Oid4vciState {
        Oid4vciState::new(
            test_key_pair(),
            HOLDER_DID.to_string(),
            kid(HOLDER_DID),
            Client::builder().no_proxy().build().unwrap(),
            store,
            issuer_url,
        )
    }

    fn offer_json(issuer: &str) -> String {
        json!({
            "credential_issuer": issuer,
            "credential_configuration_ids": [CONFIG_ID],
            "grants": {
                "urn:ietf:params:oauth:grant-type:pre-authorized_code": {
                    "pre-authorized_code": PRE_AUTH_CODE
                }
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn test_redeem_stores_credential_and_proves_holder_key() {
        let (server, seen_proof) = spawn_issuer().await;
        let issuer = server.url(ISSUER_PATH);

        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(FileCredentialStore::new(temp.path().to_path_buf()));
        let wallet = wallet(store.clone(), issuer.clone());

        let stored = wallet.redeem(&offer_json(&issuer)).await.expect("redeems");

        // The issuer recorded against allowed_issuers is the VC's `iss` (a DID),
        // not the HTTPS origin the credential came from.
        assert_eq!(stored.issuer, ISSUER_DID);
        // The DCP scope query matches on this exactly, so it must be the advertised
        // scope rather than the OID4VCI configuration id.
        assert_eq!(stored.credential_type, CREDENTIAL_SCOPE);
        assert_eq!(store.list().await.unwrap().len(), 1);

        // The proof must be a holder-signed OID4VCI proof carrying the issuer's nonce.
        let proof = seen_proof.lock().unwrap().clone().expect("proof captured");
        let header = decode_header(&proof).expect("proof header");
        assert_eq!(header.typ.as_deref(), Some(PROOF_TYP));
        assert_eq!(header.kid.as_deref(), Some(kid(HOLDER_DID).as_str()));

        let claims =
            jsonwebtoken::dangerous::insecure_decode::<Value>(&proof).expect("proof claims");
        assert_eq!(claims.claims["nonce"], NONCE);
        assert_eq!(claims.claims["aud"], issuer.trim_end_matches('/'));
    }

    #[tokio::test]
    async fn test_redeem_accepts_offer_url_forms() {
        let (server, _) = spawn_issuer().await;
        let issuer = server.url(ISSUER_PATH);
        let encoded = utf8_percent_encode(&offer_json(&issuer), NON_ALPHANUMERIC).to_string();

        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(FileCredentialStore::new(temp.path().to_path_buf()));
        let wallet = wallet(store, issuer.clone());

        let offer_url = format!("openid-credential-offer://?credential_offer={encoded}");
        assert!(wallet.redeem(&offer_url).await.is_ok(), "query form redeems");
    }

    #[tokio::test]
    async fn test_redeem_refuses_offer_from_another_issuer() {
        let (server, _) = spawn_issuer().await;
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(FileCredentialStore::new(temp.path().to_path_buf()));

        // Configured for a different issuer than the offer names.
        let wallet = wallet(store.clone(), "http://issuer.example:9999".to_string());

        let err = wallet
            .redeem(&offer_json(&server.url(ISSUER_PATH)))
            .await
            .expect_err("offer from an unconfigured issuer is refused");
        assert!(
            err.to_string().contains("not the configured issuer"),
            "unexpected error: {err}"
        );
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_redeem_requires_pre_authorized_grant() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(FileCredentialStore::new(temp.path().to_path_buf()));
        let wallet = wallet(store, String::new());

        let offer = json!({
            "credential_issuer": "http://issuer.example",
            "credential_configuration_ids": [CONFIG_ID],
            "grants": { "authorization_code": { "issuer_state": "x" } }
        })
        .to_string();

        let err = wallet.redeem(&offer).await.expect_err("no pre-auth grant");
        assert!(
            err.to_string().contains("pre-authorized code grant"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_well_known_urls_try_both_placements() {
        // RFC 8615: the well-known segment follows the authority and the issuer's own
        // path is appended after it. The appended form comes second as a fallback.
        assert_eq!(
            well_known_urls("http://issuer:7002/openid4vci", "openid-credential-issuer"),
            vec![
                "http://issuer:7002/.well-known/openid-credential-issuer/openid4vci",
                "http://issuer:7002/openid4vci/.well-known/openid-credential-issuer",
            ]
        );
        // A bare-origin issuer keeps the plain form, with or without a trailing slash,
        // and the two placements coincide so there is nothing to fall back to.
        assert_eq!(
            well_known_urls("http://issuer:7002", "openid-credential-issuer"),
            vec!["http://issuer:7002/.well-known/openid-credential-issuer"]
        );
        assert_eq!(
            well_known_urls("http://issuer:7002/", "oauth-authorization-server"),
            vec!["http://issuer:7002/.well-known/oauth-authorization-server"]
        );
    }

    /// Some issuers sign with ES256K, which `jsonwebtoken` cannot name. Both readers
    /// must still see the claims, or the credential lands with the issuer URL in place
    /// of its DID and matches no entry in `allowed_issuers`.
    #[test]
    fn test_claims_are_read_from_an_es256k_credential() {
        let jwt = unsigned_jwt(
            json!({ "alg": "ES256K", "typ": "JWT" }),
            json!({
                "iss": "did:web:wallet.demo.walt.id:wallet-api:registry:portal",
                "vc": { "type": ["VerifiableCredential", "IdentityCredential"] }
            }),
        );

        assert_eq!(
            credential_issuer_did(&jwt).as_deref(),
            Some("did:web:wallet.demo.walt.id:wallet-api:registry:portal")
        );
        assert_eq!(
            credential_type_from_vc(&jwt).as_deref(),
            Some("identity_credential")
        );
    }

    #[test]
    fn test_credential_type_ignores_the_base_type() {
        let jwt = unsigned_jwt(
            json!({ "alg": "ES256" }),
            json!({ "vc": { "type": ["VerifiableCredential"] } }),
        );

        assert_eq!(credential_type_from_vc(&jwt), None);
    }

    fn unsigned_jwt(header: Value, claims: Value) -> String {
        let segment = |value: Value| URL_SAFE_NO_PAD.encode(value.to_string());
        format!("{}.{}.signature", segment(header), segment(claims))
    }

    #[test]
    fn test_credential_response_shapes() {
        let string_form: CredentialResponse =
            serde_json::from_value(json!({ "credential": "jwt-a" })).unwrap();
        assert_eq!(string_form.into_jwt().unwrap(), "jwt-a");

        let array_form: CredentialResponse =
            serde_json::from_value(json!({ "credentials": ["jwt-b"] })).unwrap();
        assert_eq!(array_form.into_jwt().unwrap(), "jwt-b");

        let object_form: CredentialResponse =
            serde_json::from_value(json!({ "credentials": [{ "credential": "jwt-c" }] })).unwrap();
        assert_eq!(object_form.into_jwt().unwrap(), "jwt-c");

        let empty: CredentialResponse = serde_json::from_value(json!({})).unwrap();
        assert!(empty.into_jwt().is_err());
    }
}
