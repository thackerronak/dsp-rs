# AGENTS.md

A Rust implementation of the [Dataspace Protocol (DSP)](https://docs.internationaldataspaces.org/)
connector (Eclipse EDC ecosystem). Single binary (`dsp-rs`) speaking DSP 2025-1:
catalog, contract negotiation, and transfer. Rust edition 2024, axum + tokio, ES256
JWT auth, file-based storage. Authentication is a **native, self-contained Rust DCP
(Decentralized Claims Protocol) wallet** — issuer, holder, and verifier behind one
`did:web`; there is no external wallet/verifier dependency.

## Setup

Requires a local Rust toolchain (`cargo`). If missing: `brew install rust` or
[rustup](https://rustup.rs). `./build.sh` builds a self-contained Docker image that
compiles the binary inside a multi-stage build.

## Build & test

```bash
cargo check
cargo test
cargo test <name>                 # single test

./build.sh                        # build the connector:latest docker image
```

Run against the TCK harness:

```bash
RUST_LOG=debug CONFIG_PATH="tck/config.json" cargo run --bin dsp-rs --features tck
```

Config is loaded from `CONFIG_PATH` (default `config.json`) — schema is
`Configuration` in `src/connector/app_state.rs`. `docker-compose/` runs a two-party
native-DCP demo; see `docker-compose/SETUP.md` and `USAGE.md`.

## Feature flags

| Feature | Effect |
|---|---|
| `echo` (default) | in-process echo target for the reverse proxy |
| `tck` | swaps auth for TCK-harness stubs |
| `keygen` | builds the `keygen` bin |

## Code structure

```
src/
├── main.rs            # entrypoint, AppError → HTTP mapping
├── connector/         # start(), build_app()/webserver() route tree, Configuration, AppState, KeyPair
├── auth/              # identity/trust: Authenticator, AuthClaims, DidDocument
│   ├── backend.rs     # AuthBackend trait (get_token / did_document / router / extra_routes)
│   └── backends/native.rs   # NativeBackend: the symmetric DCP wallet
├── dcp/               # native DCP wallet internals
│   ├── model.rs       # DCP message serde
│   ├── si_token.rs    # Self-Issued ID Token build/validate + ReplayCache + DidResolver
│   ├── sts.rs         # Secure Token Service (POST /token)
│   ├── holder.rs      # Credential Service (/api/credentials/v1, incl. presentations/query)
│   ├── issuer.rs      # Issuer Service (/api/issuance/v1)
│   ├── verifier.rs    # presentation pull + VP/VC validation
│   ├── scope.rs       # scope grammar + ES256 JWT-VP minting
│   └── resolver.rs    # HttpDidResolver (did:web over HTTP)
├── catalog/ negotiation/ transfer/   # DSP protocol handlers (consumer.rs / provider.rs)
├── policy_engine/     # policy evaluation
├── reverse_proxy/     # /pull data-plane proxy
├── store/             # Store trait + FileStore; CredentialStore + FileCredentialStore (0600)
└── model/             # DSP wire types (serde)
```

Handlers hang off a single `AppState<T: Store>`; each sub-router gets a projected
state via axum `FromRef` (e.g. `AppStateAuthentication`). Auth is reached at three
points: outbound `authenticator.get_token`, inbound `AuthClaims` bearer validation
(`auth/extractor.rs`), and `GET /.well-known/did.json`. `Authenticator` is a thin
delegate over an `Arc<dyn AuthBackend>` plus the shared `KeyPair` (token mint/decode);
`NativeBackend` owns the wallet, serves a constructed DID document with
`CredentialService`/`IssuerService` `service[]` entries, mounts `POST /auth/token`
(via `router`) and the credential/issuance/STS routes (via `extra_routes`), and drives
the synchronous DCP presentation pull.

## Auth flow (native DCP)

Token exchange is a synchronous **pull**: the token requester mints a Self-Issued ID
Token and `POST`s it to the peer's `POST /auth/token`; the peer validates it, resolves
the requester's `CredentialService`, `POST`s a scope `PresentationQueryMessage`,
validates the returned JWT-VP + JWT-VC (`issuer ∈ allowed_issuers`), maps the
`identity` credential via `derive_access_token`, and returns the DSP access token in
one round trip. Trust is config, not architecture: `allowed_issuers` names who may
issue the presented credential (default bilateral — each party issues the other).

## Conventions

- Default visibility `pub(crate)`; the crate is one binary.
- `anyhow::Result` internally; handlers return `Result<_, AppError>` (`main.rs`).
- Mirror existing `#[cfg(feature = "tck")]` guards when touching auth; native-DCP
  integration tests are gated `#[cfg(all(test, not(feature = "tck")))]` since tck
  stubs the auth subsystem.
- Add to `AppState` via a projection struct + `FromRef`, not ad-hoc fields.
- Router tests: `tower::ServiceExt::oneshot` (no TCP bind); `tempfile` for fixtures.
  Cross-connector integration tests use loopback `TcpListener` + `build_app`.
- DSP wire types: explicit serde `rename`/`camelCase`, matching the spec JSON.
- No secrets in logs or URLs.
