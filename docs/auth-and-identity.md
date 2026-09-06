# Stage 2 — Auth & Identity (show ID at the door) 🪪

_Part of the [DSP guide](README.md) · Stage 2 of 4_

The problem: the two parties share **no password/API key**. So the buyer proves
identity with a **Verifiable Credential issued by a trusted party** — like showing a
passport instead of a secret handshake.

Each connector runs a **native DCP wallet** built in: it is its own issuer, holder,
and verifier behind a single `did:web`. There is no external wallet or verifier
service.

## Three building blocks

| DSP concept | Real-world | Code |
|-------------|-----------|------|
| **DID** + **`did.json`** | Your public passport page + seal (public key) | `auth::did`, `NativeBackend::did_document`, `resolve_did_web` |
| **Verifiable Credential** in the **credential store** | A government ID card in your pocket | `src/dcp/holder.rs`, `src/store/credential_store.rs` |
| **Verifier** | The guard who checks your ID is genuine | `src/dcp/verifier.rs`, `POST /auth/token` |

`did:web` means each party **hosts its own identity page** at its own domain —
`did:web:party-b-connector%3A3000` decodes to
`http://party-b-connector:3000/.well-known/did.json` (`%3A` = `:`),
per `resolve_did_web` (`src/connector/utils.rs`). The document also advertises the
connector's `CredentialService` and `IssuerService` under `service[]`.

## The endpoints

| Endpoint | Meaning |
|----------|---------|
| `GET /.well-known/did.json` | "Here's my public identity page + public key + services" |
| `POST /auth/token` | "Here's my Self-Issued ID Token — verify my VP and give me an access token" |
| `POST /api/credentials/v1/presentations/query` | Holder side: "Here's a VP for the requested scope" |
| `POST /token`, `/api/issuance/v1/*`, `/api/credentials/v1/*` | STS, issuer, and holder credential services |

## The flow (`NativeBackend`, `src/auth/backends/native.rs`)

Native DCP is a synchronous **pull** — the verifier fetches the presentation, so
there is no session or polling:

1. **mint + call** — InsureCo mints a **Self-Issued ID Token** (`iss == sub ==` its
   own DID, `aud =` AutoParts) and `POST`s it to AutoParts' `POST /auth/token`.
2. **pull** — AutoParts validates the SI token (signature via InsureCo's DID doc,
   `aud`, `jti` replay, expiry), resolves InsureCo's `CredentialService`, and `POST`s
   a scope `PresentationQueryMessage` for `identity_credential`. InsureCo answers
   inline with an ES256 **JWT-VP** wrapping its JWT-VC.
3. **verify + badge** — AutoParts validates the VP and VC (VC `issuer ∈
   allowed_issuers`), maps the credential via `derive_access_token`, and returns an
   `access_token` **in the same response**.

## What's in the badge (the access token)

A JWT AutoParts signs with its own ES256 key (`derive_access_token`, `src/auth/mod.rs`):
```json
{ "iss": "did:web:party-a-connector%3A3000",   // issued BY the host
  "sub": "did:web:party-b-connector%3A3000",   // belongs TO InsureCo
  "email": "ops@insureco.example",
  "country": "EU" }                             // attribute that feeds policy checks
```
InsureCo sends this as `Authorization: Bearer ...` on every call. AutoParts'
`AuthClaims` extractor (`src/auth/extractor.rs`) verifies its own signature; no/bad
badge → `401`.

## Two superpowers vs. an API key

- **Mutual & symmetric** — when AutoParts later *calls back*
  ([Stage 3](negotiation.md) & [Stage 4](transfer.md)), it runs the same dance
  against InsureCo. Both sides prove identity, and both run the full wallet.
- **Trustless onboarding** — InsureCo never registered with AutoParts; it just
  presented a credential from an issuer AutoParts already trusts (`allowed_issuers`).

> 🚗 **Catena-X:** the presented credential is the **Membership + BPN** credential;
> the attribute baked into the badge is the **BPN**, which then drives access
> policies (e.g. *"only BPNL…BMW may pull this"*). `allowed_issuers` holds the
> Catena-X Issuer's DID.

---

**Prev:** [← Producer & Discovery](producer-and-discovery.md) · **Next:** [Stage 3 — Negotiation →](negotiation.md) · [Index](README.md)
