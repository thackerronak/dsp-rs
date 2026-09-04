# DSP Connector — Documentation

A guide to the Dataspace Protocol (DSP 2025-1) as implemented by this connector,
told through one running real-world example. Start here, then follow the reading
order below — each page is short and focused.

> **New to dataspaces?** Read [Why DSP?](why-dsp.md) first. Just want the endpoint
> mechanics? Jump to [Catalog](catalog.md) and follow the arrows at the bottom of
> each page.

---

## The running example (used throughout)

A B2B data marketplace — one seller, one buyer:

| Role | Example | Identity (DID) | Address |
|------|---------|----------------|---------|
| **Provider** (seller) | `AutoParts GmbH` — publishes EV battery test results | `did:web:party-a-connector%3A3000` | `http://party-a-connector:3000` |
| **Consumer** (buyer) | `InsureCo` — an insurer that wants that dataset | `did:web:party-b-connector%3A3000` | `http://party-b-connector:3000` |

A **connector** is each company's automated "trade desk" that speaks DSP on its
behalf. Every connector is **both** a producer and a consumer — the roles are
symmetric (in the demo, party-a and party-b each federate the other).

All DSP messages use the JSON-LD context
`https://w3id.org/dspace/2025/1/context.jsonld` under the base path `/api/2025/1`.

---

## The four-stage journey

| Stage | Doc | Endpoints | Real-world | Output |
|-------|-----|-----------|-----------|--------|
| **1. Browse** | [Catalog](catalog.md) | `/catalog/request` | Read the shop's catalogue | A dataset `@id` + its policy |
| **2. Show ID** | [Auth & Identity](auth-and-identity.md) | `/auth/verify_me`, `/auth/status`, `/.well-known/did.json` | Prove identity → get a badge | A signed access **token** |
| **3. Sign contract** | [Negotiation](negotiation.md) | `/negotiations/*` | Sign a contract by registered mail | A finalized **Agreement ID** |
| **4. Collect goods** | [Transfer](transfer.md) | `/transfers/*`, `/pull` | Pick up with a temporary locker key | The **actual data** |

The other half of the story — how a producer publishes data and how consumers
find it — is in [Producer & Discovery](producer-and-discovery.md).

---

## Reading order

1. [Why DSP? (and why a central Issuer isn't "centralized")](why-dsp.md)
2. [Stage 1 — Catalog](catalog.md)
3. [Producer & Discovery](producer-and-discovery.md)
4. [Stage 2 — Auth & Identity](auth-and-identity.md)
5. [Stage 3 — Negotiation](negotiation.md)
6. [Stage 4 — Transfer](transfer.md)

**Reference:**
- [Glossary](glossary.md) — DID, VC, VP, ODRL, DCAT, PID, EDR, BPN, …
- [Limitations, security & production-readiness](limitations.md)
- [Sequence diagrams](diagrams/README.md) — Mermaid (renders on GitHub) + PlantUML

---

## Running it

See [`../docker-compose/SETUP.md`](../docker-compose/SETUP.md) and
[`../docker-compose/USAGE.md`](../docker-compose/USAGE.md) for a runnable
two-participant demo of the full flow.
