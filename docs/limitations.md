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
| **HTTPS not enforced** | DID resolution passes `enforce_https = false`, and the demo runs over `http://`. Enforce HTTPS (and certificate validation) before production. | `src/auth/mod.rs` (`resolve_did_web(did_web, false)`) |
| **Single verification key assumed** | The JWT `kid` header isn't used when selecting the DID's verification key — it assumes one key per DID. Multi-key DIDs / key rotation aren't handled. | `src/auth/mod.rs` |
| **Credential types are hardcoded** | Only a fixed `identity_credential` is requested at `verify_me`; credentials can't yet be selected per-offer, so a policy can't demand a specific framework/use-case credential per dataset. | `src/auth/mod.rs` |

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
| **No global discovery** | Producers must be pre-configured in `federation`; there's no discovery service. See [Producer & Discovery](producer-and-discovery.md). | `src/catalog/sync.rs` |
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
