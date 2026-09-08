# AGENTS.md

A Rust implementation of the [Dataspace Protocol (DSP)](https://docs.internationaldataspaces.org/)
connector (Eclipse EDC ecosystem). Single binary (`dsp-rs`) speaking DSP 2025-1:
catalog, contract negotiation, and transfer. Rust edition 2024, axum + tokio, ES256
JWT auth, file-based storage. Authentication is a **native wallet** — holder,
verifier, and issuer behind one `did:web`, with no external wallet or verifier
service. The wallet speaks two credential-exchange protocols: **DCP** (Decentralized
Claims Protocol, issuance and presentation) and **OID4VC** (issuance only, OID4VCI).

## Setup

Requires a local Rust toolchain (`cargo`). The `third_party/dsp-spec` submodule holds
the JSON schemas the validator loads, so init it before running tests:

```sh
git submodule update --init third_party/dsp-spec
```

## Build & test

```sh
cargo check
cargo check --features tck          # TCK build must keep compiling
cargo test
cargo test <name>                   # single test

./build.sh                          # connector:latest image (compiles inside the image)

# end-to-end over the Docker demo (needs Docker; builds the image, ~1 min)
cargo test --test e2e_docker -- --ignored --nocapture
```

`tests/e2e_docker.rs` is `#[ignore]`d so `cargo test` stays green without Docker. It
brings the compose stack up from cold, issues a credential from the walt.id issuer over
OID4VCI, exchanges DCP tokens both ways, then negotiates and pulls the dataset — the
whole of `SETUP.md` and `USAGE.md`. It tears the stack down and deletes the seeded
credentials, so don't run it against a stack you are using by hand.

`docker-compose/` runs a two-party demo; see `docker-compose/SETUP.md` and `USAGE.md`.
Config is loaded from `CONFIG_PATH` (default `config.json`) — the schema is
`Configuration` in `src/connector/app_state.rs`.

Run against the TCK harness:

```sh
RUST_LOG=debug CONFIG_PATH="tck/config.json" cargo run --bin dsp-rs --features tck
```

## Feature flags

| Feature | Effect |
|---|---|
| `echo` (default) | in-process echo target for the reverse proxy |
| `tck` | swaps auth for TCK-harness stubs |
| `keygen` | builds the `keygen` bin |

## Layering — the one rule to preserve

```
connector  ->  auth  ->  wallet  ->  shared
```

`src/wallet/` must depend **only** on `src/shared/` and external crates — never on
`crate::connector` or `crate::auth`. A test in `src/wallet/mod.rs` enforces this,
walking the whole tree including the protocol submodules. It exists so the wallet can
be lifted into its own crate, or its own process, as a move rather than a redesign.
If you need something from the connector inside `wallet`, move it into `shared/`
instead of reaching across.

## Code structure

```
src/
├── main.rs            # entrypoint, AppError -> HTTP mapping
├── shared/            # KeyPair, DID-document types, did:web helpers
├── wallet/            # the wallet, depends only on shared/
│   ├── store.rs       # credential store (0600 files) — protocol-agnostic
│   ├── did.rs         # DidResolver trait + did:web over HTTP
│   ├── dcp/           # Decentralized Claims Protocol 1.0
│   │   ├── si_token.rs  # Self-Issued ID Token build/validate, ReplayCache
│   │   ├── holder.rs    # Credential Service (/api/credentials/v1)
│   │   ├── issuer.rs    # Issuer Service (/api/issuance/v1)
│   │   ├── verifier.rs  # presentation pull + VP/VC validation
│   │   ├── scope.rs     # scope grammar + JWT-VP minting
│   │   ├── sts.rs       # Secure Token Service (opt-in, /api-internal/sts)
│   │   └── model.rs     # DCP message serde
│   └── oid4vc/        # OpenID for Verifiable Credentials
│       └── vci.rs     # OID4VCI: redeem a credential offer from an external issuer
├── auth/              # Authenticator (concrete), /auth/token verifier, AuthClaims
├── connector/         # start(), build_app(), Configuration, AppState
├── catalog/ negotiation/ transfer/   # DSP handlers (consumer.rs / provider.rs)
├── policy_engine/     # policy evaluation
├── reverse_proxy/     # /pull data-plane proxy
├── store/             # Store trait + FileStore
└── model/             # DSP wire types (serde)
```

Handlers hang off a single `AppState<T: Store>`; each sub-router gets a projected
state via axum `FromRef` (e.g. `AppStateAuthentication`). Auth is reached at three
points: outbound `authenticator.get_token`, inbound `AuthClaims` bearer validation
(`auth/extractor.rs`), and `GET /.well-known/did.json`.

`Authenticator` is **concrete**. There is one wallet implementation, so there is no
backend trait and nothing downcasts in the request path. If a second identity backend
ever appears, introduce the trait then — with two implementers in hand.

## Auth flow

Token exchange is a synchronous **pull**: the requester mints a Self-Issued ID Token
and POSTs it to the peer's `POST /auth/token`; the peer validates it, resolves the
requester's `CredentialService`, POSTs a scope `PresentationQueryMessage`, validates
the returned JWT-VP and JWT-VC (`issuer` in `allowed_issuers`), maps the `identity`
credential via `derive_access_token`, and returns the DSP access token — one round
trip, no session, no polling.

Trust is configuration, not architecture: `allowed_issuers` names who may issue the
presented credential. See `docs/limitations.md` for the known gaps.

## Conventions

- Default visibility `pub(crate)`; the crate is one binary.
- `anyhow::Result` internally; handlers return `Result<_, AppError>` (`main.rs`).
- Mirror existing `#[cfg(feature = "tck")]` guards when touching auth.
- Add to `AppState` via a projection struct + `FromRef`, not ad-hoc fields.
- Router tests: `tower::ServiceExt::oneshot` (no TCP bind); `tempfile` for fixtures.
  Cross-connector tests use loopback `TcpListener` + `build_app`.
- DSP wire types: explicit serde `rename`/`camelCase`, matching the spec JSON.
- No secrets in logs or URLs.
