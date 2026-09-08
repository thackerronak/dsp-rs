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
