# Implementation Plan — Native Rust DCP wallet as a second authentication backend

_Status: PLAN ONLY (no code yet). Author target: `dsp-rs` maintainers._

## 1. Goal & guiding principle

Add a **native, Rust-based DCP (Decentralized Claims Protocol) wallet** as a
**selectable second authentication backend**, alongside the existing walt.id
integration.

**Principle: pluggable, not a rewrite.**
- walt.id stays and remains the **default** — zero regression for existing
  deployments.
- An operator chooses the backend **per connector, via config** (`auth.backend`).
- Both backends implement the same internal traits, so the DSP layer
  (catalog / negotiation / transfer) is completely unaffected.

This mirrors the pattern the repo already uses for the `Store` trait
(`FileStore` today, a DB tomorrow).

The native implementation is a Rust port of the two reference Python services:
- `dcp_holder_wallet.py` → **Holder Credential Service** (storage + issuance client)
- `dcp_issuer_service.py` → **Issuer Service** (separate binary)

## 2. Scope & the "presentation gap"

DCP has three roles: **Issuer**, **Holder**, **Verifier**. The reference Python
covers **issuance + storage**; a full auth backend also needs
**presentation + verification**. In `dsp-rs` today, walt.id is used on *both*
sides of auth (`src/auth/mod.rs`):

- as the **holder wallet** — `get_token` asks it to *present*
  (`POST /wallet/{id}/credentials/present`);
- as the **verifier** — `verify_me` / `status` delegate VP verification.

Therefore, to be a real "wallet the user can choose," the native backend must
eventually cover the presentation/verification leg too. This plan sequences that
as **Phase 4** so issuance+storage can land first.

| DCP capability | Reference Python | This plan |
|---|---|---|
| Issuer service | ✅ `dcp_issuer_service.py` | Phase 3 (separate binary) |
| Holder credential storage + request | ✅ `dcp_holder_wallet.py` | Phase 2 |
| Presentation query/response + verification | ❌ (not in the files) | Phase 4 |

> Note: DCP presentation is **not** OpenID4VP. The native backend deliberately
> speaks a different presentation protocol than today's walt.id path; the two
> backends are alternatives, not layers.

## 3. Reuse map (Python → existing Rust)

Most primitives already exist — the new work is DCP messages, endpoints,
credential storage, and the pluggability seam.

| Python (DCP) | Reuse in `dsp-rs` | New work |
|---|---|---|
| `generate_keypair` (P-256) | `src/tools/keygen.rs`, `KeyPair` | — |
| `create_jwt` / decode (ES256) | `KeyPair::encode` / `jsonwebtoken` | thin `si_token` helper |
| `resolve_did_doc_url` (`%3A`, localhost→http) | `src/connector/utils.rs::resolve_did_web` | — |
| `GET /.well-known/did.json` | `src/auth/mod.rs::did` | add `service[]` entries |
| DID signature verification | `Authenticator::load_verify_did` | reuse for si_token/VC verify |
| `httpx` client | `reqwest` + `post_request` / `get_request` | — |
| FastAPI routers | `axum` routers | new route modules |
| `stored_credentials` JSON file | file-store pattern (`src/store/`) | `CredentialStore` trait + file impl |

## 4. Target architecture

### 4.1 Trait seam (Phase 0)

Introduce backend-agnostic traits; `Authenticator` calls the traits, never
walt.id directly.

```rust
// Holder side: hold VCs and present them when challenged.
#[async_trait]
trait Wallet: Send + Sync {
    async fn present(&self, challenge: PresentationQuery)
        -> anyhow::Result<VerifiablePresentation>;
    async fn store_credential(&self, vc: CredentialContainer) -> anyhow::Result<()>;
    async fn list_credentials(&self) -> anyhow::Result<Vec<CredentialContainer>>;
    async fn request_credential(&self, issuer_did: &str, cred_type: &str)
        -> anyhow::Result<()>;
}

// Verifier side: validate a presented VP into claims used for the access token.
#[async_trait]
trait CredentialVerifier: Send + Sync {
    async fn verify(&self, vp: VerifiablePresentation)
        -> anyhow::Result<VerifiedClaims>;
}
```

Implementations:
- `WaltIdWallet` / `WaltIdVerifier` — extract today's HTTP-client logic from
  `Authenticator` (no behavior change).
- `NativeWallet` / `NativeVerifier` — the new Rust DCP implementation.

`Authenticator` holds `Arc<dyn Wallet>` + `Arc<dyn CredentialVerifier>`, chosen at
startup from config.

### 4.2 Config selection (how the user picks a wallet)

Backend is chosen per connector in `config.json`. Backward compatible: if
`auth.backend` is absent, default to `waltid` and read the existing top-level
fields.

```jsonc
"auth": {
  "backend": "waltid",              // "waltid" (default) | "native"
  "allowed_issuers": ["did:web:issuer-did-server"],

  "waltid": {                        // used when backend = "waltid"
    "wallet_url":  "http://party-a-wallet:7005",
    "wallet_id":   "569ad2cc-...",
    "verifier_url":"http://party-a-verifier:7003",
    "issuer_url":  "http://issuer:7002"
  },

  "native": {                        // used when backend = "native"
    "credential_store_path": "./data/credentials",
    "credential_service_path": "/api/credentials/v1"
    // signing reuses the connector's existing private_key_pem / did:web
  }
}
```

### 4.3 Feature flag

Gate the native code behind a Cargo feature so default builds are unchanged:
`--features native-dcp`. The Issuer is a separate bin gated the same way
(`--features dcp-issuer`).

### 4.4 Module layout

```
src/
├── auth/
│   ├── mod.rs                 # Authenticator now delegates to Wallet/Verifier traits
│   ├── wallet.rs              # Wallet + CredentialVerifier traits + VerifiedClaims
│   └── backends/
│       ├── waltid.rs          # WaltIdWallet / WaltIdVerifier (extracted from today)
│       └── native.rs          # NativeWallet / NativeVerifier
├── dcp/                       # shared DCP protocol layer
│   ├── mod.rs
│   ├── model.rs               # message + VC/VP structs (serde)
│   ├── si_token.rs            # self-issued token build + validate
│   ├── did.rs                 # DID document w/ service[] (CredentialService/IssuerService)
│   └── credential_service.rs  # holder-side axum router (/api/credentials/v1)
└── bin/
    └── dcp_issuer.rs          # standalone Issuer service (separate binary)

src/store/
└── credential_store.rs        # CredentialStore trait + FileCredentialStore
```

## 5. DCP protocol details to implement

### 5.1 Messages (`src/dcp/model.rs`)

Serde structs with `@context = ["https://w3id.org/dspace-dcp/v1.0/dcp.jsonld"]`:
- `CredentialOfferMessage` { issuer, credentials: [CredentialObject] }
- `CredentialRequestMessage` { holderPid, credentials: [{ id }] }
- `CredentialMessage` { issuerPid, holderPid, status, credentials: [CredentialContainer] }
- `CredentialRequestStatusMessage` { issuerPid, holderPid, status }  // PROCESSING | ISSUED | …
- `IssuerMetadata` { issuer, credentialsSupported: [CredentialObject] }
- `CredentialObject` { id, credentialType, profile, bindingMethods, schema, … }
- `CredentialContainer` { credentialType, payload (VC-JWT), format, type }
- `PresentationQuery` / `VerifiablePresentation` (Phase 4)

### 5.2 Self-issued token (`src/dcp/si_token.rs`)

- **Build** (holder → issuer): ES256 JWT, claims `{ iss = sub = holder DID,
  aud = issuer DID, iat, nbf, exp (~300s), jti, token }`, signed with the
  connector's key (`kid = <did>#keys-1`).
- **Validate** (issuer side, per DCP §4.3.3): `iss == sub`; `aud == issuer DID`
  (normalise `%3A`); `exp` not passed; signature verified against `iss` DID
  document (reuse `load_verify_did`). Pass-through of the inner `token` for
  delivery auth.

### 5.3 DID document (`src/dcp/did.rs`)

Extend the connector's `/.well-known/did.json` with a `service[]` array:
- Holder role → `{ type: "CredentialService", serviceEndpoint: <base>/api/credentials/v1 }`
- Issuer bin → `{ type: "IssuerService", serviceEndpoint: <base>/api/issuance/v1 }`

(The Python advertises exactly these; the Issuer resolves the holder's
`CredentialService` and vice-versa.)

## 6. Endpoints to implement

### 6.1 Holder Credential Service (Phase 2) — mounted on the connector

| Endpoint | From Python | Purpose |
|---|---|---|
| `POST /api/credentials/v1/offers` | `receive_offer` | receive `CredentialOfferMessage`; capture bearer token; async-trigger request |
| `POST /api/credentials/v1/credentials` | `receive_delivered_credential` | **Storage API**: validate (token, holderPid, type, VC subject DID, expiry, credentialStatus) and store the VC |
| `GET  /api/credentials/v1/wallet` | `get_wallet` | list stored credentials |
| `POST /api/credentials/v1/request` | `trigger_credential_request` | holder-initiated request (resolve issuer metadata → catalog id) |
| _internal_ | `send_credential_request` | resolve issuer DID→IssuerService, mint si_token, POST `/credentials`, follow `Location`, poll status |

### 6.2 Issuer Service (Phase 3) — separate `dcp_issuer` binary

| Endpoint | From Python | Purpose |
|---|---|---|
| `GET  /.well-known/did.json` | `get_issuer_did` | issuer DID doc (IssuerService) |
| `GET  /api/issuance/v1/metadata` | `get_issuer_metadata` | `IssuerMetadata` (supported creds) |
| `POST /api/issuance/v1/credentials` | `handle_credential_request` | validate si_token, create+sign VC, store status session, `201 + Location`, async deliver |
| `GET  /api/issuance/v1/credentials/requests/{issuer_pid}` | `get_credential_request_status` | status polling (PROCESSING/ISSUED) |
| `POST /api/issuance/v1/offer` | `trigger_credential_offer` | issuer-initiated offer to a holder |
| _internal_ | `deliver_credential` | resolve holder DID→CredentialService, POST `CredentialMessage` |

## 7. Credential storage (Phase 2)

- `CredentialStore` trait: `store`, `list`, `get`, (later) `delete`.
- `FileCredentialStore` — JSON files under `auth.native.credential_store_path`,
  reusing the atomic temp-file+rename pattern from `src/store/file_store.rs`.
- DB backend (SQLite/Postgres) deferred; the trait keeps it a drop-in later.
- **Key custody note:** the wallet signs with the connector's existing
  `private_key_pem`. For production, plan a follow-up to move key storage to an
  OS keystore / KMS / HSM (out of scope for the first cut).

## 8. Integration with the existing auth path

- `verify_me` / `status` / `get_token` in `src/auth/mod.rs` keep their signatures
  but call the `Wallet` / `CredentialVerifier` traits.
- `derive_access_token` stays as-is (it maps `VerifiedClaims` → the connector's
  ES256 access token). The native verifier just produces `VerifiedClaims` from a
  DCP VP instead of from a walt.id verifier response.
- `allowed_issuers` enforcement applies to **both** backends (native verifier
  checks the VC issuer DID against the list, exactly as walt.id's `allowed-issuer`
  policy does today).

## 9. Phased delivery

| Phase | Deliverable | Depends on |
|---|---|---|
| **0** | `Wallet`/`CredentialVerifier` traits; extract `WaltIdWallet`/`WaltIdVerifier`; config `auth.backend` (default waltid, no behaviour change) | — |
| **1** | `src/dcp/` core: message models, `si_token`, DID `service[]` | 0 |
| **2** | Native Holder Credential Service + `FileCredentialStore` (issuance client + storage) | 1 |
| **3** | `dcp_issuer` separate binary (issue + deliver + status) | 1 |
| **4** | DCP Presentation/Verification: `NativeWallet::present` + `NativeVerifier::verify`; wire into `verify_me`/`get_token` | 2 |
| **5** | Config/feature wiring, docs, demo compose profile for the native backend | 2–4 |
| **6** | Tests + interop (native↔native, native↔walt.id), full DSP e2e with `backend=native` | 2–5 |

After Phase 3 you can already **issue and store** credentials natively (replacing
the walt.id issuer for seeding). After Phase 4 a connector can run auth with **no
walt.id at all**.

## 10. Testing & interop strategy

- **Unit:** DCP message serde round-trips; `si_token` validation truth table
  (iss==sub, aud, exp, bad signature); DID `service[]` resolution.
- **Integration:** native holder ↔ native issuer, reproducing the Python flows
  (`--offer` issuer-initiated and `/request` holder-initiated).
- **Interop (conformance):** native holder ↔ **walt.id** issuer, and walt.id
  holder ↔ **native** issuer, to prove wire compatibility.
- **End-to-end:** run the existing DSP demo (catalog→negotiate→transfer) with
  `auth.backend = native` on one or both parties.
- **Regression:** default (`waltid`) path unchanged; existing tests green.

## 11. Risks & open questions

- **Presentation protocol details.** The reference files don't include the DCP
  presentation flow; confirm the exact DCP Presentation Query/Response shape
  (Catena-X DCP spec) before Phase 4.
- **VC formats.** Python issues `vc11-sl2021/jwt` (JWT VC). SD-JWT VCs (as used by
  the current walt.id identity credential) may need separate handling if required.
- **Revocation.** `credentialStatus` is parsed but not enforced in the Python;
  decide whether the native verifier must fetch/verify status lists.
- **Key custody.** File-based key + credential storage is fine for dev; production
  needs a hardening pass (encrypted-at-rest / KMS).
- **DID `service[]` coexistence.** Ensure adding services to the connector's DID
  doc doesn't disturb the existing DSP auth that resolves the same document.

## 12. Rough effort (engineering estimate, excl. review)

| Phase | Estimate |
|---|---|
| 0 — trait seam + config | 1–2 days |
| 1 — DCP core | 1–2 days |
| 2 — holder credential service + store | 2–3 days |
| 3 — issuer binary | 2 days |
| 4 — presentation/verification | 3–5 days (spec-dependent) |
| 5 — wiring/docs/compose | 1–2 days |
| 6 — tests/interop | 2–3 days |

Issuance+storage MVP (Phases 0–3): ~1.5 weeks. Full auth replacement
(through Phase 6): ~3–4 weeks.

---

_This is a design plan only; no code is changed. See the two reference services
for the exact message shapes and validation steps to port._
