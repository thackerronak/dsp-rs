# Stage 2 — Auth & Identity (show ID at the door) 🪪

_Part of the [DSP guide](README.md) · Stage 2 of 4_

The problem: the two parties share **no password/API key**. So the buyer proves
identity with a **Verifiable Credential** — like showing a passport instead of a
secret handshake.

Each connector carries its own **DCP wallet**: it holds its credentials, presents
them, and verifies the credentials others present, all behind a single `did:web`.
There is no external wallet or verifier service.

## Three building blocks

| DSP concept | Real-world | Code |
|-------------|-----------|------|
| **DID** + **`did.json`** | Your public passport page + seal (public key) | `auth::did`, `Authenticator::did_document`, `src/shared/did.rs` |
| **Verifiable Credential** in the **credential store** | A government ID card in your pocket | `src/wallet/dcp/holder.rs`, `src/wallet/store.rs` |
| **Verifier** | The guard who checks your ID is genuine | `src/wallet/dcp/verifier.rs`, `src/auth/token.rs` |

`did:web` means each party **hosts its own identity page** at its own domain —
`did:web:party-b-connector%3A3000` decodes to
`http://party-b-connector:3000/.well-known/did.json` (`%3A` = `:`), per
`resolve_did_web` (`src/shared/did.rs`). The document also advertises the
connector's `CredentialService` and `IssuerService` under `service[]`, which is
how the other side knows where to ask for a presentation.

## The endpoints

| Endpoint | Meaning |
|----------|---------|
| `GET /.well-known/did.json` | "Here's my identity page, public key and services" |
| `POST /auth/token` | "Here's my Self-Issued ID Token — verify me and give me a badge" |
| `POST /api/credentials/v1/presentations/query` | Holder side: "Here's a VP for the scope you asked for" |
| `POST /api/credentials/v1/request` | Ask an issuer for a credential (see [below](#getting-a-credential)) |
| `POST /api/issuance/v1/credentials` | Issuer side: mint and push a credential to a holder |

## The flow (one round trip)

Unlike a challenge/response with polling, the DCP exchange here is a **synchronous
pull**. InsureCo wants a badge from AutoParts:

1. **Self-Issued ID Token** — InsureCo signs a short-lived JWT with its own key
   (`iss` = `sub` = InsureCo's DID, `aud` = AutoParts' DID, plus a `jti`) and POSTs
   it as a Bearer token to AutoParts' `/auth/token` (`build_si_token`,
   `src/wallet/dcp/si_token.rs`).
2. **AutoParts validates it** — resolves InsureCo's DID document, checks the
   signature, `aud`, expiry, and that the `jti` hasn't been seen before
   (`ReplayCache`).
3. **AutoParts pulls a presentation** — it reads InsureCo's `CredentialService`
   from that DID document and POSTs a `PresentationQueryMessage` asking for scope
   `org.eclipse.dspace.dcp.vc.type:identity_credential`. InsureCo answers with a
   **JWT-VP** wrapping its credential (`src/wallet/dcp/verifier.rs`, `src/wallet/dcp/scope.rs`).
4. **AutoParts validates VP + VC** — the VP must be signed by InsureCo and
   addressed to AutoParts; the VC inside must be signed by an issuer listed in
   **`allowed_issuers`**. Then it maps the credential onto claims
   (`derive_access_token`) and returns the token.

Note the direction: the **verifier** reaches back into the requester's wallet. No
session, no polling.

## What's in the badge (the access token)

A JWT AutoParts signs with its **token** key (`private_key_pem`) — a different key
from the one in its DID document, because nobody else ever verifies this token:
```json
{ "iss": "did:web:party-a-connector%3A3000",   // issued BY the host
  "sub": "did:web:party-b-connector%3A3000",   // belongs TO InsureCo
  "email": "ops@insureco.example",
  "country": "EU" }                             // attribute that feeds policy checks
```
InsureCo sends this as `Authorization: Bearer ...` on every call. AutoParts'
`AuthClaims` extractor (`src/auth/extractor.rs`) verifies its own signature; no/bad
badge → `401`. The credential key (`wallet.private_key_pem`) signs the things peers
*do* verify — SI tokens, presentations, issued credentials — and is the one published
in `did.json`.

## Getting a credential

Before any of the above works, a connector must actually **hold** a credential.
Two native paths exist today, both DCP:

```sh
# holder-initiated: ask an issuer for a credential
curl -X POST http://localhost:23000/api/credentials/v1/request \
  -H 'content-type: application/json' \
  -d '{"issuerDid":"did:web:party-a-connector%3A3000"}'

# issuer-initiated: push an offer to a holder
curl -X POST http://localhost:13000/api/issuance/v1/offer \
  -H 'content-type: application/json' \
  -d '{"holderDid":"did:web:party-b-connector%3A3000"}'
```

An external issuer that speaks **OID4VCI** is redeemed instead — the connector runs
the pre-authorized code flow itself, signing the holder proof with its own key so the
private key never leaves it:

```sh
curl -X POST http://localhost:23000/api-internal/credentials/redeem \
  -H 'content-type: application/json' \
  -d '{"offerUrl":"openid-credential-offer://?credential_offer=..."}'
```

Unlike the DCP paths this one is **synchronous** — it returns the stored credential's
id and issuer, or an error. An offer naming an issuer other than the configured
`issuer_url` is refused.

The two DCP calls above return `202` — their delivery is an **asynchronous push**: the issuer mints the
credential and POSTs it to the holder's `/api/credentials/v1/credentials`. Poll
`GET /api/credentials/v1/credentials` until it lands.

> The credential must be signed by an issuer in the verifier's `allowed_issuers`,
> or step 4 above rejects it. Self-issued credentials therefore only work if the
> parties list each other's DIDs. See [Limitations](limitations.md) for the
> intended external-issuer path.

## Two superpowers vs. an API key

- **Mutual & symmetric** — when AutoParts later *calls back*
  ([Stage 3](negotiation.md) & [Stage 4](transfer.md)), it runs the same exchange
  against InsureCo. Both sides prove identity.
- **Trustless onboarding** — InsureCo never registered with AutoParts; it just
  presented a credential from an issuer AutoParts already trusts.

> 🚗 **Catena-X:** the presented credential is the **Membership + BPN** credential;
> the attribute baked into the badge is the **BPN**, which then drives access
> policies (e.g. *"only BPNL…BMW may pull this"*). `allowed_issuers` holds the
> Catena-X Issuer's DID.

---

**Prev:** [← Producer & Discovery](producer-and-discovery.md) · **Next:** [Stage 3 — Negotiation →](negotiation.md) · [Index](README.md)
