# Stage 2 — Auth & Identity (show ID at the door) 🪪

_Part of the [DSP guide](README.md) · Stage 2 of 4_

The problem: the two parties share **no password/API key**. So the buyer proves
identity with a **credential issued by a trusted third party** — like showing a
passport instead of a secret handshake.

## Three building blocks

| DSP concept | Real-world | Code |
|-------------|-----------|------|
| **DID** + **`did.json`** | Your public passport page + seal (public key) | `src/auth/mod.rs` (`did`), `resolve_did_web` |
| **Verifiable Credential** in a **Wallet** | A government ID card in your pocket | wallet, `/wallet/{id}/credentials/present` |
| **Verifier** | The guard who checks your ID is genuine | walt.id verifier behind `verify_me` |

`did:web` means each party **hosts its own identity page** at its own domain —
`did:web:party-b-connector%3A3000` decodes to
`http://party-b-connector:3000/.well-known/did.json` (`%3A` = `:`),
per `resolve_did_web` (`src/connector/utils.rs`).

## The 3 endpoints

| Endpoint | Meaning |
|----------|---------|
| `GET /.well-known/did.json` | "Here's my public identity page + public key" |
| `POST /auth/verify_me` | "Prove who you are" → guard issues a challenge |
| `GET /auth/status/{session_id}` | "Have I passed? If so, give me my badge" |

## The flow (`get_token`, `src/auth/mod.rs`)

Run before any protected call:

1. **verify_me** — InsureCo POSTs `{ did_web }` to AutoParts. AutoParts resolves
   InsureCo's DID, opens a verification session demanding an `identity_credential`
   with policies (signature valid, not expired, **issued by an allowed issuer**),
   and returns `{ session_id, openid4vp_url }`.
2. **present** — InsureCo's wallet presents its Verifiable Presentation to
   AutoParts' verifier (OpenID4VP).
3. **status** — InsureCo polls until success; AutoParts then **prints the badge**
   (`derive_access_token`) and returns an `access_token`.

## What's in the badge (the access token)

A JWT AutoParts signs with its own ES256 key (`src/auth/mod.rs`):
```json
{ "iss": "did:web:party-a-connector%3A3000",   // issued BY the host
  "sub": "did:web:party-b-connector%3A3000",   // belongs TO InsureCo
  "email": "ops@insureco.example",
  "country": "EU" }                             // attribute that feeds policy checks
```
InsureCo sends this as `Authorization: Bearer ...` on every call. AutoParts'
`AuthClaims` extractor (`src/auth/extractor.rs`) verifies its own signature; no/bad
badge → `401`. Tokens are **cached until expiry**.

## Two superpowers vs. an API key

- **Mutual & symmetric** — when AutoParts later *calls back*
  ([Stage 3](negotiation.md) & [Stage 4](transfer.md)), it runs the same dance
  against InsureCo. Both sides prove identity.
- **Trustless onboarding** — InsureCo never registered with AutoParts; it just
  presented a credential from an issuer AutoParts already trusts (`allowed_issuers`).

> 🚗 **Catena-X:** the presented credential is the **Membership + BPN** credential;
> the attribute baked into the badge is the **BPN**, which then drives access
> policies (e.g. *"only BPNL…BMW may pull this"*). `allowed_issuers` holds the
> Catena-X Issuer's DID.

---

**Prev:** [← Producer & Discovery](producer-and-discovery.md) · **Next:** [Stage 3 — Negotiation →](negotiation.md) · [Index](README.md)
