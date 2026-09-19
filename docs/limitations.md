# Limitations, security & production-readiness

_Part of the [DSP guide](README.md) · Reference_

This connector implements the full DSP 2025-1 happy path and runs the two-party
demo end to end. Before relying on it in production, be aware of the following —
each item is grounded in a `TODO`/`FIXME`/`unimplemented!` marker in the source, so
treat this as a "known gaps" list, not a criticism.

> This reflects the state of the code at the time of writing; check the source for
> the current status.

## Security

| Item | Detail | Where |
|------|--------|-------|
| **HTTPS not enforced** | DID resolution passes `enforce_https = false`, and the demo runs over `http://`. Enforce HTTPS (and certificate validation) before production. | `src/wallet/did.rs` (`resolve_did_web(did, false)`) |
| **Single verification key assumed** | The JWT `kid` header isn't used when selecting the DID's verification key — it assumes one key per DID. Multi-key DIDs / key rotation aren't handled. | `src/shared/did.rs` (`decoding_key_for`) |
| **Credential type is hardcoded** | The verifier always asks for scope `…dcp.vc.type:identity_credential`; credentials can't yet be selected per-offer, so a policy can't demand a specific framework/use-case credential per dataset. | `src/wallet/dcp/verifier.rs` (`query_peer_presentation`) |
| **Credential-acquisition triggers are unauthenticated** | `POST /api/credentials/v1/request` and `POST /api/issuance/v1/offer` take a JSON body and no SI token, unlike every other holder/issuer endpoint, yet sit on public paths. Anyone who can reach the connector can make it request credentials from an arbitrary DID (an outbound POST carrying a connector-signed SI token) or mint and push a credential to an arbitrary holder. Move them behind `/api-internal` or require an SI token. | `src/wallet/dcp/holder.rs` (`trigger_request`), `src/wallet/dcp/issuer.rs` (`trigger_offer`) |
| **Replay protection is per-process** | The `jti` replay cache is an in-memory map. It prunes expired entries, so it does not grow without bound, but it is lost on restart and is not shared between replicas — so a replayed SI token would be accepted by a second instance. | `src/wallet/dcp/si_token.rs` (`ReplayCache`) |
| **One key for all credential roles** | The wallet signs with a single key covering `authentication`, `assertionMethod` and `capabilityInvocation` in the DID document, plus issuer signing. A key per role would limit the blast radius of a compromise. | `src/wallet/key_pair.rs` |

## Identity & credentials

| Item | Detail | Where |
|------|--------|-------|
| **External issuance verified against the live issuer** | Exercised end to end against `waltid/issuer-api2:1.0.0`: the image does advertise `IdentityCredential_jwt_vc_json` with format `jwt_vc_json`, both connectors redeem a pre-authorized offer, and `POST /auth/token` returns `200`. Getting there needed four client-side corrections and one issuer-config correction — see the notes under this table. | `src/wallet/oid4vc/vci.rs`, `docker-compose/issuer/config/` |
| **SD-JWT credentials unsupported** | `validate_vc` reads a W3C JWT-VC (`vc.credentialSubject`). An IETF SD-JWT VC — disclosures, `_sd` digests, `cnf` key binding — is neither verified nor presented. | `src/wallet/dcp/verifier.rs` |
| **Credential delivery is async with no status** | `/request` and `/offer` return `202`; the credential arrives later via a push to the holder's `/credentials`. There is no way to ask whether a given request succeeded, so callers poll `GET /api/credentials/v1/credentials`. | `src/wallet/dcp/holder.rs` |
| **Self-issuance is the default trust model** | Each connector is also an issuer. That is convenient for a demo but means trust is configuration (`allowed_issuers`), not architecture. A real dataspace puts a third party in the issuer role. | `src/wallet/dcp/issuer.rs` |
| **walt.id needs a W3C-shaped profile** | `waltid/issuer-api2:1.0.0` writes a profile's `credentialData` into the `vc` object more or less verbatim. A flat `credentialData` therefore yields a token with no `vc.credentialSubject` and no top-level `exp`, neither of which `validate_vc` accepts. The profile has to nest the claims under `credentialSubject` itself, and set `issuanceDate`/`expirationDate` so the issuer emits the registered `nbf`/`exp` claims. This is a property of the image, not of the connector. | `docker-compose/issuer/config/issuer2-profiles.conf` |

### What the live issuer required

The image behaves differently from the OID4VCI draft the client was written against.
For anyone pointing this connector at another issuer, these were the mismatches:

- **The issuer identifier carries a path** (`http://issuer:7002/openid4vci`), so its
  metadata is at `/.well-known/openid-credential-issuer/openid4vci` — the well-known
  segment goes after the authority, not after the path (RFC 8615). Concatenating
  `{issuer}/.well-known/...` 404s.
- **`pre-authorized_grant_anonymous_access_supported` is `false`**, so the token request
  must carry a `client_id`; without one the issuer answers `invalid_client`.
- **The token response carries no `c_nonce`.** The nonce comes from the advertised
  `nonce_endpoint` instead.
- **The credential request needs OID4VCI draft-15 `proofs`** (keyed by proof type), not
  the older single `proof` object; with only `proof` the issuer answers
  `invalid_proof: Credential request is missing proofs`.
- **The advertised `scope`, not the credential configuration id, is the DCP credential
  type.** The configuration id (`IdentityCredential_jwt_vc_json`) names a format; the
  scope (`identity_credential`) is what a DCP presentation query matches on.

## Interoperating with the Java EDC

The demo runs a third party — an Eclipse EDC control plane, data plane and IdentityHub —
and exchanges data with it in both directions (`docker-compose/SETUP.md`,
`tests/e2e_party_c.rs`). Getting there surfaced these, which are worth knowing before
pointing this connector at any other implementation.

| Item | Detail | Where |
|------|--------|-------|
| **A callback address must be complete** | A peer's advertised DSP address is used verbatim, and this connector's own now carries the version path. Publishing a bare host and letting the sender append its own version path only works between two connectors that agree on that path. | `src/connector/app_state.rs` (`callback_address`), `src/negotiation/mod.rs` (`api_address`) |
| **A peer's DID cannot be derived from its address** | The peer DID comes from the authenticated claims (provider side) or the federation configuration (consumer side). Party C's DID resolves to its IdentityHub, on a different host and port from its DSP endpoint — deriving `did:web:` from the address gets it wrong. | `src/negotiation/provider.rs`, `src/connector/internal_api.rs` |
| **The scope grammar carries an operation** | A presentation query asks for `…vc.type:identity_credential:read`, and an inbound scope's trailing `read`/`*`/`all` is stripped before matching. The Java EDC's Credential Service rejects a type scope with no operation. | `src/wallet/dcp/scope.rs` |
| **Two implementations match a query on different things** | This connector matches a presentation query against the type it stored a credential under; the Java EDC matches against the `type` array inside the credential. The demo therefore uses one string, `identity_credential`, in both places — including in the walt.id profile. | `src/wallet/dcp/holder.rs`, `docker-compose/issuer/config/issuer2-profiles.conf` |
| **A presentation carries no `sub`** | `validate_vp` requires `iss` and checks `sub` only when present. The holder is the presentation's issuer; the Java EDC omits `sub` entirely. | `src/wallet/dcp/verifier.rs` |
| **An issued credential needs `issuanceDate`** | Credentials this connector mints carry `issuanceDate`, `expirationDate`, an `id` and the holder in `credentialSubject.id`. A credential without `issuanceDate` is rejected outright by the Java EDC. | `src/wallet/dcp/issuer.rs` (`mint_identity_credential`) |
| **Credential formats are a fixed vocabulary** | Issued credentials go on the wire as `VC1_0_JWT`. A holder that parses the format field — the Java EDC's IdentityHub does — rejects anything outside `VC1_0_LD`/`VC1_0_JWT`/`VC2_0_JOSE`/`VC2_0_SD_JWT`/`VC2_0_COSE`. | `src/wallet/dcp/issuer.rs` (`CREDENTIAL_FORMAT`) |
| **Countering an identical offer stalls an EDC consumer** | When the consumer asks for exactly what was advertised, the provider agrees immediately instead of counter-offering. A counter-offer is legal DSP, but the Java EDC's management API has no action for a consumer to accept one, so the negotiation would never move. | `src/negotiation/provider.rs` |
| **A DSP filter is opaque** | `CatalogRequestMessage.filter` is an array of arbitrary JSON, as the schema says. It was typed as an array of strings, which rejected the Java EDC's criterion objects. The store still ignores it. | `src/model/catalog.rs`, `src/store/mod.rs` |
| **`dspace:endpoint` is dropped by the peer** | A transfer's data address is sent with the endpoint as `dspace:endpoint`, per the DSP context. This EDC release reads the endpoint from its own namespace, so a consumer there receives the token but no endpoint and has to address the provider directly. The reverse direction is unaffected. | `src/model/transfer.rs`, `docker-compose/USAGE.md` |

Two things on the other side of the wire that this connector cannot fix, recorded so they
are not re-diagnosed: a DID document's `authentication`/`assertionMethod` entries must be
reference strings or full verification methods — a bare `{"id": …}` is rejected by the Java
EDC (`docker-compose/issuer/did.json`) — and party C's IdentityHub has been seen to answer a
presentation query with `401 No verification method found with key ID 'keys-1'` for a key its
peer does publish, which a restart of IdentityHub clears.

## DCP TCK conformance

Run against `eclipse-dataspacetck/dcp-tck` (`tck/dcp-rs.tck.properties`, see its comments
for how): 85 of 117 test cases pass. That run found and fixed five real conformance bugs,
none related to party C — the DCP TCK exercises the wallet's Credential Service, Issuer
Service and verifier directly, none of which the two-party demo's own tests happen to probe
this hard:

| Item | Detail | Where |
|------|--------|-------|
| **Issuer metadata had the wrong shape** | `GET /metadata` returned an object missing `"type": "IssuerMetadata"`, and its `CredentialObject` entries were missing `"type": "CredentialObject"`, had no `profile`, and an empty `bindingMethods` — the TCK's own metadata schema requires all four, and a peer resolving credentials by id needs them too. | `src/wallet/dcp/issuer.rs` (`identity_credential_object`, `metadata`) |
| **A delivered credential's container was parsed too strictly** | `CredentialContainer.type` was a required field, but the TCK's own seed `CredentialMessage`s omit it (the container's type is implied by context) — every seeding call this connector received failed to parse, with a `422` that then cascaded into every test that depended on seeded credentials being present. | `src/wallet/dcp/model.rs` (`CredentialContainer`) |
| **The credential-status endpoint required no authentication at all** | `GET /requests/{issuer_pid}` had no `authenticate()` call, so any caller — no token, an expired one, one signed by someone else — could poll any request's status, including one made under a different holder's identity. | `src/wallet/dcp/issuer.rs` (`request_status`) |
| **SI token validation didn't check `nbf` or `iat`, and let `exp` slide 60s past** | `jsonwebtoken`'s `Validation` defaults to `validate_nbf: false` (silently off) and a 60s `leeway` on `exp`; nothing validates `iat` at all regardless of config. An SI token is minted and used within the same request, so none of that slack serves a purpose here — it only masked stale or backdated tokens the TCK's negative tests specifically construct. | `src/wallet/dcp/si_token.rs` (`validate_si_token`) |
| **`allowed_issuers` didn't include this connector's own DID** | `tck/config.json` initially pointed `allowed_issuers` at a guessed external DID. With it fixed to this connector's own DID (self-issued credentials, matching the demo's own `docker-compose/party-a` convention), two presentation-verifier tests that need a *trusted* credential to reach validation at all started passing — and, in the same run, exposed the next item, which that wrong config had been silently hiding behind an unrelated 401. | `tck/config.json` (`allowed_issuers`) |

What's left, in four groups:

1. **A real, security-relevant gap in verifying a presented credential**, only visible once
   `allowed_issuers` is correct (above) and the verifier actually reaches this code instead
   of rejecting everything on trust alone: the verifier accepts a presentation carrying an
   **expired** credential, a **revoked/suspended** one (revocation isn't implemented at
   all — see "No revocation status list" above), one **not yet valid**, one whose
   `credentialSubject.id` **doesn't match the presenting holder**, one with a **violated
   schema**, and a presentation that contains a **different credential than the scope
   requested** or is **missing a requested one**. `validate_vc` (`src/wallet/dcp/verifier.rs`)
   checks the JWT envelope's `iss`/`sub`/`exp` and the issuer trust list, but never reads the
   VC-1.1 body's own `credentialSubject.id` or `expirationDate`/`issuanceDate`, never checks
   revocation, and nothing cross-checks a presentation's credential types against what the
   scope actually asked for. This is real verification logic to add, not a config or parsing
   fix. Covers `5.4.2.1`, `5.4.2.3`, `5.4.2.4`, `5.4.3`, `5.4.3.6` (both cases), `5.4.3.7`,
   and the schema-violation case.
2. **Already known** — the `kid`-header-ignored gap (see "Single verification key assumed"
   above) fails the TCK's "rejects a token whose `kid` resolves to no verification method"
   cases outright, since this connector doesn't look at `kid` in the first place.
3. **A structural mismatch between this connector's architecture and the TCK's harness
   model, not a bug in either.** The TCK models holder, issuer and verifier as three
   separate parties, each with its own key it can override via
   `dataspacetck.key.{holder,issuer}` — except `.key.verifier`, which does not exist; the
   TCK always signs as verifier with a key it generates itself. This connector is one
   identity playing all three roles. Setting `dataspacetck.did.verifier` to this
   connector's DID (required so the `aud` on inbound presentation-verifier requests
   matches) means the TCK's own verifier-signed queries to this connector's Credential
   Service — the `presentation.cs` package — can never carry a signature this connector
   can validate, since there is no way to supply the matching private key. Symmetrically,
   a delivery-tracking assertion in `issuance.issuer` checks the TCK's own in-process
   mock object rather than a real HTTP round trip, which only sees a delivery when
   `dataspacetck.did.holder` is the TCK's own generated identity rather than this
   connector's real one — the opposite of what `issuance.cs`/`presentation.cs` need. No
   single value for these properties satisfies every package. Affects `5.4.1.2`, `4.3.3`
   (verifier `jti`), `6.1`, `6.4.1`, `6.5.1`, `6.5.2`.
4. **Real, unimplemented business rules**, independent of the above: the Credential
   Service accepts a `CredentialMessage` for any `holderPid` rather than only ones with a
   pending request, doesn't validate the `status` field against a known set, and doesn't
   reject an empty `credentials` array; the Issuer Service doesn't prevent issuing twice
   for the same `holderPid` or reject an empty `credentials` array on a request; a sparse
   `CredentialOfferMessage` (credentials referenced by `id` only, no type) isn't resolved
   against the issuer's own metadata, so it can't be accepted — a real DCP capability this
   connector doesn't have, not a parsing gap.

## Protocol coverage

| Item | Detail | Where |
|------|--------|-------|
| **Offer-by-reference unimplemented** | Only *concrete* offers are supported; a referenced offer hits `unimplemented!()` and would panic. | `src/negotiation/provider.rs`, `consumer.rs` |
| **PUSH transfer incomplete** | `HttpData-PULL` is the fully-wired path; the PUSH `dataAddress` handling is marked FIXME. | `src/transfer/consumer.rs`, `src/transfer/mod.rs` |
| **Agreement `assigner`/`assignee` sometimes empty** | Several agreement constructions leave these fields empty (FIXME). | `src/store/file_store.rs`, `src/negotiation/provider.rs`, `src/transfer/consumer.rs` |
| **Auto-accept / auto-start** | The consumer auto-accepts offers and the provider auto-starts transfers in places (marked as open questions), rather than gating on an explicit business decision. | `src/negotiation/consumer.rs`, `src/transfer/provider.rs` |

## Discovery & state

| Item | Detail | Where |
|------|--------|-------|
| **No global discovery** | Producers must be pre-configured in `federation` by DID; their endpoints are resolved from the DID document, but there is no service that tells you which DIDs exist. See [Producer & Discovery](producer-and-discovery.md). | `src/catalog/sync.rs` |
| **`DataService` follows the spec example, not the spec schema** | `did-service-schema.json` constrains every `serviceEndpoint` to match `https?://.*?/catalog$`, yet the spec's own `DataService` example points at `/.well-known/dspace-version` and fails that pattern. We publish and consume the example's shape, since the version endpoint is what discovery needs. A peer that follows the schema literally will not interoperate. | `src/auth/mod.rs` (`build_local_did_document`), `src/catalog/sync.rs` (`dsp_root_from_version_endpoint`) |
| **Terminated negotiations re-picked-up** | A code comment flags that terminated negotiations get picked up by the pending query — a potential reprocessing loop to verify. | `src/store/file_store.rs` |
| **Unstable catalog id & ignored filter** | The catalog `@id` is a fresh UUID each request (FIXME: should be stable), and `get_datasets`' `filter` argument is currently ignored. | `src/catalog/mod.rs`, `src/store/file_store.rs` |
| **File-based store** | The default `Store` is file-backed; consider concurrency/atomicity and a database backend for scale. The `Store` trait already abstracts this. | `src/store/file_store.rs` |
| **No revocation status list** | Issued credentials carry no revocation mechanism (no `bitstringstatuslist`/`revocationlist2020` status entry), so a revoked credential is indistinguishable from a valid one. The DCP TCK's revocation test cases fail on this regardless of configuration. | `src/wallet/dcp/issuer.rs` (`mint_identity_credential`) |

## Ergonomics

| Item | Detail | Where |
|------|--------|-------|
| **Internal API returns raw state** | `/api-internal/negotiate/{pid}` and `/transfer/{pid}` return raw JSON rather than a clean `pending/failed/succeeded` status enum (TODO). | `src/connector/internal_api.rs` |
| **Version path hardcoded** | Only `2025-1` is supported; the metadata-driven path selection is a TODO. | `src/negotiation/provider.rs`, `src/transfer/provider.rs` |

---

None of these block the demo, but the **security** items (HTTPS enforcement,
`kid`/key handling) and the **panics** (offer-by-reference) are the first things to
address for any real deployment. Fixes belong in their own reviewed changes, not in
documentation.

---

**Prev:** [← Glossary](glossary.md) · [Index](README.md)
