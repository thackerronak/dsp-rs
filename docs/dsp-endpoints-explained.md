# DSP Endpoints Explained — a beginner's walkthrough

This guide explains the Dataspace Protocol (DSP 2025-1) endpoints implemented by
this connector, using one running real-world example. It complements the
[sequence diagram](diagrams/README.md) and the runnable demo in
[`docker-compose/USAGE.md`](../docker-compose/USAGE.md).

> **TL;DR** — A connector lets two companies that have *never met* discover data,
> prove who they are, sign an enforceable digital contract, and move data
> **directly** (peer-to-peer), while the seller keeps control of *who* uses the
> data and *how*. The endpoints below are the four stages of that story.

---

## The running example

A B2B data marketplace — one seller, one buyer:

| Role | Example | Identity (DID) | Address |
|------|---------|----------------|---------|
| **Provider** (seller) | `AutoParts GmbH` — publishes a dataset of EV battery test results | `did:web:party-a-connector%3A3000` | `http://party-a-connector:3000` |
| **Consumer** (buyer) | `InsureCo` — an insurer that wants that dataset | `did:web:party-b-connector%3A3000` | `http://party-b-connector:3000` |

A **connector** is each company's automated "trade desk" that speaks DSP on its
behalf. The journey has four stages, each an endpoint group:

1. **Browse the shop** → Catalog
2. **Show ID at the door** → Auth / DID
3. **Haggle & sign the contract** → Negotiation
4. **Pick up the goods** → Transfer

All DSP messages use the JSON-LD context
`https://w3id.org/dspace/2025/1/context.jsonld` and live under the base path
`/api/2025/1`.

---

## Why a protocol at all? Can't they just connect directly?

For **two** parties, a direct API is simpler. But real data ecosystems have
**hundreds** of parties, each both buyer and seller. Direct integration means an
N×N explosion (~5,000 bespoke pipes for 100 companies), each with its own auth,
its own contract format (a PDF no software can enforce), its own data format.

DSP replaces those pipes with **one shared language**: build to the spec once,
trade with anyone else who did the same — no prior relationship required. Like
**shipping containers** — standardize once, and any container fits any ship,
crane, and truck worldwide.

What a direct connection **cannot** give you, and DSP does:

1. **Interoperability without prior agreement** — join the network, not each peer.
2. **Data sovereignty** — the seller keeps control *after* handoff (machine-readable
   policies travel with the deal and are enforced by the connector).
3. **Software-executable contracts** — negotiation yields a signed digital
   agreement; no agreement → no data.
4. **Decentralized trust** — no central broker holds your data; each party proves
   its own identity cryptographically.
5. **Auditability & symmetry** — every step is an explicit, logged, standardized
   message; the same connector buys and sells.

### "But Catena-X has a central Issuer — isn't that centralized?"

Two different layers:

- **Identity issuance** (who is a legitimate member?) → needs a **central trust
  anchor** (an Issuer). Minimal, one-time, at onboarding.
- **Data exchange** (how does data flow?) → **fully decentralized, peer-to-peer.**

Analogy: your **passport** is issued centrally (a government), but you then
**travel freely** — the passport office isn't the airline, hotel, or customs
officer for every trip, and never sees your journeys. Same as HTTPS: a small set
of trusted **Certificate Authorities** vouch for domains, but your traffic never
routes through them.

In this codebase the trust anchor is a **list** — `allowed_issuers`
(`src/auth/mod.rs`) — so a dataspace can trust multiple issuers. Everything about
the *data* (catalog, negotiation, transfer, enforcement) is decentralized; the
Issuer only vouches for identity and never sees the data.

> **Catena-X mapping.** The trusted Issuer is the central **Catena-X Issuer
> service**, operated by the network's Operating Company (e.g. Cofinity-X) under
> the governance of the *Catena-X Automotive Network e.V.* It issues
> **Membership**, **BPN** (Business Partner Number), and use-case **Framework**
> credentials. `did:web:issuer.example.com` + `BpnCredential` in a DCP diagram is
> exactly this Issuer. **DCP** = how you *get* your Catena-X ID; **DSP** (this
> repo) = how you *use* it to trade data.

---

## Stage 1 — Catalog (browse the shop) 🛒

Read-only browsing. Nothing is reserved, nothing signed, no data moves.
Defined in `src/catalog/mod.rs`.

| Endpoint | Meaning |
|----------|---------|
| `POST /api/2025/1/catalog/request` | "Show me your whole catalogue" |
| `GET /api/2025/1/catalog/datasets/{id}` | "Show me the detail page for this one product" |

**Request** (InsureCo → AutoParts):
```http
POST /api/2025/1/catalog/request
Authorization: Bearer <token>

{ "@context": ["https://w3id.org/dspace/2025/1/context.jsonld"],
  "@type": "CatalogRequestMessage",
  "filter": [] }
```

**Response** — a `dcat:Catalog`. For each dataset, like a shelf label:
- **`@id`** — the product's barcode (quoted later to negotiate).
- **`odrl:hasPolicy`** — the usage terms / "price tag" (e.g. *"use only, EU only"*).
- **`dcat:distribution` → `accessService`** — which counter to collect from.

In this repo, InsureCo usually discovers AutoParts' catalog ahead of time via
**catalog sync** (`src/catalog/sync.rs`) and caches it locally as a *federated
dataset* — see the next section.

---

## The producer side — publishing data & how consumers discover it

Everything above is from the *consumer's* point of view. Here's the other half:
how a **producer** (AutoParts) offers data, and how a **consumer** (InsureCo)
finds out it exists.

### Publishing: you describe datasets, the catalog is assembled on the fly

There is **no "create catalog" endpoint**. A producer just **publishes dataset
descriptor files** in its connector's `data/datasets/` folder; the catalog is
generated per request from whatever files are present. Real demo file
(`docker-compose/party-a/connector/data/datasets/urn-uuid-3afeadd8-...json`):

```json
{
  "dataset": {
    "@id": "urn:uuid:3afeadd8-...-8394a8836d57",     // the product barcode
    "@type": "Dataset",
    "hasPolicy": [{                                    // public usage terms ("price tag")
      "@type": "Offer",
      "@id": "urn:uuid:2828282:...a88",
      "permission": [{ "action": "use",
        "constraint": [{ "leftOperand": "spatial",
                         "operator": "isPartOf",
                         "rightOperand": "_:EU" }] }]   // "EU only"
    }],
    "distribution": [{ "@type": "Distribution", "format": "HttpData-PULL",
                       "accessService": { "endpointURL": "https://provider-a.com/connector" } }]
  },
  "remoteAddress": { "url": "http://localhost:4000/api", "passHeaders": true }  // ← the REAL backend
}
```

Two halves:
- **`dataset`** — the *public* description buyers see (id, policy, distribution).
- **`remoteAddress`** — the *private* wiring: where the **actual bytes** live (a DB,
  an internal API, a file server). The `/pull` proxy forwards here in Stage 4
  (`get_dataset_target`, `src/store/file_store.rs`). **Buyers never see this URL.**

When a consumer calls `POST /catalog/request`, the handler (`catalog_request`,
`src/catalog/mod.rs`):
1. reads **every** file in `data/datasets/` (`get_datasets`, `src/store/file_store.rs`);
2. **rewrites** each distribution's endpoint to the connector's *own* address (the
   pickup counter is the connector, not the raw backend);
3. wraps them in a fresh `RootCatalog` and returns it.

> The datasets on disk are the **source of truth**; the "catalog" is a **live view**
> assembled at request time. Add a file → it's instantly in the catalog. (`file_store`
> is one implementation of the `Store` trait; a real deployment could use a database
> without changing any endpoint.)

### Discovery: how a consumer knows a producer exists

**The DSP protocol does not define global discovery.** It only says *how to ask an
already-known producer for its catalog* — it assumes you already have the
counterparty's connector URL. In this repo, that gap is filled by
**pre-configured federation + periodic catalog sync** (a pull model):

1. **Configure who to sync from.** The consumer's `config.json` lists known
   producers in a `federation` block
   (`docker-compose/party-b/connector/config.json`):
   ```json
   "federation": { "party-a": { "remote_address": "http://party-a-connector:3000" } }
   ```
   An admin put that address there — *that* is "how it knows."
2. **Sync on a timer.** On startup `catalog_sync` (`src/catalog/sync.rs`) spawns one
   task per federated connector. Each task periodically: checks the producer's
   version (`/.well-known/dspace-version`), gets a token (Stage 2), calls the
   producer's `POST /catalog/request`, and **caches** the datasets locally as
   *federated datasets* (`save_federated_dataset` → `data/datasets/federated/<name>/`).
3. **Negotiate against the cache.** `POST /api-internal/negotiate {"name":"party-a"}`
   looks up the cached federated dataset and starts Stage 3.

```
Producer (party-a)                  Consumer (party-b)
  data/datasets/*.json                config.json → federation: { party-a }
       │                                       │
       │  ◄── every N sec: POST /catalog/request (catalog_sync)
       │  ──► RootCatalog ─────────────────────►│
       │                                        ▼
       │                     data/datasets/federated/party-a/*.json  (cached)
                                                │
                                   /api-internal/negotiate name=party-a
```

### Discovery is a layer *above* DSP

This repo's "configure the address" is the minimal approach. Real dataspaces add a
discovery layer so you don't hardcode every partner:

| Approach | How discovery works |
|----------|--------------------|
| **This repo** | Static `federation` config + periodic catalog sync (you must know the address) |
| **Catena-X** | Central **Discovery Finder** + **BPN Discovery Service** resolve a partner's **BPN → connector URL**, plus a **Digital Twin Registry** and federated-catalog crawlers |
| **Gaia-X / IDS** | A **Federated Catalogue / Broker** where participants register self-descriptions and others query centrally |

> **DSP** = the *bilateral* catalog protocol ("ask a **known** producer for its
> list"). **Discovery** = a *separate* concern ("find **which** producers exist and
> **where**"). Here it's config; in Catena-X it's a central Discovery Service that
> fills in the address this repo hardcodes.

---

## Stage 2 — Auth / DID (show ID at the door) 🪪

The problem: the two parties share **no password/API key**. So the buyer proves
identity with a **credential issued by a trusted third party** — like showing a
passport instead of a secret handshake.

Three building blocks:

| DSP concept | Real-world | Code |
|-------------|-----------|------|
| **DID** + **`did.json`** | Your public passport page + seal (public key) | `src/auth/mod.rs` (`did`), `resolve_did_web` |
| **Verifiable Credential** in a **Wallet** | A government ID card in your pocket | wallet, `/wallet/{id}/credentials/present` |
| **Verifier** | The guard who checks your ID is genuine | walt.id verifier behind `verify_me` |

`did:web` means each party **hosts its own identity page** at its own domain —
`did:web:party-b-connector%3A3000` decodes to
`http://party-b-connector:3000/.well-known/did.json` (`%3A` = `:`),
per `resolve_did_web` (`src/connector/utils.rs`).

| Endpoint | Meaning |
|----------|---------|
| `GET /.well-known/did.json` | "Here's my public identity page + public key" |
| `POST /auth/verify_me` | "Prove who you are" → guard issues a challenge |
| `GET /auth/status/{session_id}` | "Have I passed? If so, give me my badge" |

**The flow** (`get_token`, `src/auth/mod.rs`), run before any protected call:

1. **verify_me** — InsureCo POSTs `{ did_web }` to AutoParts. AutoParts resolves
   InsureCo's DID, opens a verification session demanding an `identity_credential`
   with policies (signature valid, not expired, **issued by an allowed issuer**),
   and returns `{ session_id, openid4vp_url }`.
2. **present** — InsureCo's wallet presents its Verifiable Presentation to
   AutoParts' verifier (OpenID4VP).
3. **status** — InsureCo polls until success; AutoParts then **prints the badge**
   (`derive_access_token`) and returns an `access_token`.

**What's in the badge** — a JWT AutoParts signs with its own ES256 key:
```json
{ "iss": "did:web:party-a-connector%3A3000",   // issued BY the host
  "sub": "did:web:party-b-connector%3A3000",   // belongs TO InsureCo
  "email": "ops@insureco.example",
  "country": "EU" }                             // attribute that feeds policy checks
```
InsureCo sends this as `Authorization: Bearer ...` on every call. AutoParts'
`AuthClaims` extractor (`src/auth/extractor.rs`) verifies its own signature; no/bad
badge → `401`. Tokens are **cached until expiry**.

Two superpowers vs. an API key:
- **Mutual & symmetric** — when AutoParts later *calls back* (Stages 3–4), it runs
  the same dance against InsureCo. Both sides prove identity.
- **Trustless onboarding** — InsureCo never registered with AutoParts; it just
  presented a credential from an issuer AutoParts already trusts
  (`allowed_issuers`).

> **Catena-X mapping.** The presented credential is the **Membership + BPN**
> credential; the attribute baked into the badge is the **BPN**, which then drives
> access policies (e.g. *"only BPNL…BMW may pull this"*). `allowed_issuers` holds
> the Catena-X Issuer's DID.

---

## Stage 3 — Negotiation (sign the contract) 🤝📝

Like signing a B2B contract **by registered mail**: each side mails a signed
letter, every letter advances a shared state machine, and each carries **case
numbers** (PIDs) so both sides match the correspondence. No Issuer involved — pure
peer-to-peer. Defined in `src/negotiation/`.

**PID = Process ID** — the reference number for one negotiation. There are two,
because each side keeps its own:
- `consumerPid` — created by the consumer at the start (its purchase-order number).
- `providerPid` — minted by the provider when it first sees the request (its
  sales-order number).

Every message carries **both**.

**State machine:**
```
REQUESTED ──► (OFFERED ──► ACCEPTED) ──► AGREED ──► VERIFIED ──► FINALIZED ✅
                                    └────────► TERMINATED ❌ (either side, any time)
```

**Endpoints — the endpoint lives on the _receiver_** (`src/negotiation/mod.rs`):

| Endpoint | Hosted by | Message received | Result |
|----------|-----------|------------------|--------|
| `POST /negotiations/request` | Provider | initial `ContractRequestMessage` | `201`, REQUESTED (mints `providerPid`) |
| `POST /negotiations/{providerPid}/request` | Provider | counter `ContractRequestMessage` | ACCEPTED |
| `POST /negotiations/{consumerPid}/offers` | Consumer | `ContractOfferMessage` | OFFERED |
| `POST /negotiations/{consumerPid}/agreement` | Consumer | `ContractAgreementMessage` | AGREED |
| `POST /negotiations/{providerPid}/agreement/verification` | Provider | `ContractAgreementVerificationMessage` | VERIFIED |
| `POST /negotiations/{consumerPid}/events` | Consumer | `ContractNegotiationEventMessage` (FINALIZED) | FINALIZED |
| `POST /negotiations/{pid}/termination` | Either | `ContractNegotiationTerminationMessage` | TERMINATED |
| `GET /negotiations/{pid}` | Either | (read current state) | — |

**Happy path:**

1. **Request** (`contract_request`, `src/negotiation/provider.rs`) — InsureCo POSTs
   a `ContractRequestMessage` with the `offer` (policy `@id` + dataset `target`) and
   a `callbackAddress`. Provider mints `providerPid`, returns `201` REQUESTED.
2. **Policy engine** (`src/negotiation/provider.rs`, `negotiate_policy`) — the
   provider compares the consumer's *requested* terms against the dataset's
   *required* policy. Compatible → advance; incompatible → auto-**TERMINATED**
   (`"policy negotiation failed"`). **This is data sovereignty enforced in code.**
3. **Agreement** — provider callbacks to the consumer's `/agreement` with a signed
   `Agreement` that has its own `@id` (needed in Stage 4), `assigner`, `assignee`,
   `target`, and `permission`/`constraint`.
4. **Verification** — consumer countersigns via `/agreement/verification` → VERIFIED.
5. **Finalized** — provider callbacks to the consumer's `/events` with
   `eventType: FINALIZED` → the `Agreement` is now binding on both connectors.

**Why it's asynchronous.** Handlers don't do the work inline — they queue the
message and immediately ACK (`200`/`201`). A **background loop** ticking every
second (`handle_negotiations` → `process_negotiations`, `src/negotiation/mod.rs`)
finds pending negotiations and `.tick()`s them, sending the next letter to the
other side's `callbackAddress`. This distributed state machine scales to many
concurrent deals without holding connections open.

**Why verify + finalize (double handshake).** Provider agrees → consumer verifies
→ provider finalizes. Both parties provably, cryptographically consented — a
**non-repudiable** contract you can't later disown.

> **Catena-X mapping.** The negotiated `offer` is a Catena-X **usage/access
> policy** (e.g. `FrameworkAgreement.traceability`, `Membership`, a BPN constraint).
> The resulting `Agreement` `@id` is the **contract agreement** stored in the EDC,
> quoted later to start a transfer.

---

## Stage 4 — Transfer (pick up the goods) 📦🔑

You've signed the contract; now collect the data. Key new idea: **control plane
vs. data plane.**

| Plane | Carries | In this repo |
|-------|---------|--------------|
| **Control plane** | coordination messages ("start", "complete", tokens, state) | `/transfers/*` |
| **Data plane** | the actual bytes of the dataset | the `/pull/*` reverse proxy |

DSP separates them: the `/transfers/*` handshake never carries data — it
*negotiates access* to it; the bytes flow through a different door (`/pull`).
Two styles exist — **PULL** (provider hands you an endpoint + token, you fetch)
and **PUSH** (provider sends to your endpoint). This repo demos `HttpData-PULL`.

**State machine:**
```
REQUESTED ──► STARTED ⇄ SUSPENDED ──► COMPLETED
                  └──────────────────► TERMINATED (either side, any time)
```

**Endpoints** (`src/transfer/mod.rs`) — again the receiver hosts each:

| Endpoint | Hosted by | Message | Result |
|----------|-----------|---------|--------|
| `POST /transfers/request` | Provider | `TransferRequestMessage` (quotes the Agreement ID) | `201`, REQUESTED |
| `POST /transfers/{consumerPid}/start` | Consumer | `TransferStartMessage` (carries `DataAddress`: endpoint + token) | STARTED |
| `POST /transfers/{consumerPid}/suspension` | Consumer | `TransferSuspensionMessage` | SUSPENDED |
| `POST /transfers/{providerPid}/completion` | Provider | `TransferCompletionMessage` | COMPLETED |
| `POST /transfers/{pid}/termination` | Either | `TransferTerminationMessage` | TERMINATED |
| `GET /transfers/{providerPid}` | Either | (read state) | — |
| `ANY /pull/*` | Provider (data plane) | the actual data request | streams bytes |

**Happy path:**

1. **Request** — InsureCo POSTs a `TransferRequestMessage` quoting
   `agreementId` (from Stage 3) and `format: "HttpData-PULL"`. **No finalized
   agreement → no transfer.** Provider returns `201` REQUESTED.
2. **Start** (`src/transfer/provider.rs`) — provider mints a **transfer token**
   scoped to this transfer (`new_transfer_token`, with `sub` = the transfer's
   `providerPid`) and callbacks to the consumer's `/start` with a `DataAddress`:
   the `/pull` endpoint + the token. State → STARTED.
3. **Pull (data plane)** — InsureCo calls `GET /pull/...` with
   `Authorization: <transfer_token>`. The reverse proxy (`src/reverse_proxy/mod.rs`,
   `find_data_asset_target`):
   - decodes the token → `sub` = `providerPid`;
   - looks up the transfer — it **must be STARTED**, else `401`;
   - resolves the `Agreement` → the **real backend dataset URL** (hidden from the
     consumer);
   - **streams** the data back (HTTP and WebSocket supported). Repeat while STARTED.
4. **Completion** — InsureCo POSTs `TransferCompletionMessage` to the provider's
   `/completion` → COMPLETED.

**Why this design = sovereignty, made real:**
- The consumer **never sees the real data URL** — only `/pull`; the proxy holds the
  true address.
- Access is **live and revocable** — every pull re-checks the STARTED state, so
  suspension/completion/termination locks the token instantly. (Contrast a plain
  download: once the file is out, it's gone.)
- The token is **scoped to one contract** (`sub → providerPid → agreement →
  dataset`); it can't reach any other dataset.
- **The agreement gates everything** — the Stage-3 contract *is* the access key.

> **Catena-X mapping.** The `DataAddress` (endpoint + token) is Catena-X's **EDR**
> (Endpoint Data Reference); `/pull` is the EDC **data plane**, `/transfers/*` the
> **control plane** — same token-gated pull.

---

## The whole journey, end to end

| Stage | Endpoints | Real-world | Output |
|-------|-----------|-----------|--------|
| **1. Browse** | `/catalog/request` | Read the shop's catalogue | A dataset `@id` + its policy |
| **2. Show ID** | `/auth/verify_me`, `/auth/status`, `/.well-known/did.json` | Prove identity → get a badge | A signed access **token** |
| **3. Sign contract** | `/negotiations/*` | Sign a contract by registered mail | A finalized **Agreement ID** |
| **4. Collect goods** | `/transfers/*`, `/pull` | Pick up with a temporary locker key | The **actual data** |

The three "why DSP not a direct API" ideas, as real code:
- **Trustless identity** → Stage 2 (`allowed_issuers`, verifiable credentials).
- **Enforceable contracts** → Stage 3 (policy engine + signed Agreement).
- **Data sovereignty** → Stage 4 (scoped, revocable, agreement-gated pull).

---

## Bonus: the internal (operator) API

Humans/operators don't speak DSP directly — they drive their own connector through
a small internal API (`src/connector/internal_api.rs`), which then performs the DSP
dance above on their behalf:

| Endpoint | Purpose |
|----------|---------|
| `POST /api-internal/negotiate` | Start a negotiation for a discovered dataset |
| `GET /api-internal/negotiate/{consumer_pid}` | Check negotiation state (→ finalized Agreement) |
| `POST /api-internal/transfer` | Start a transfer for a finalized agreement |
| `GET /api-internal/transfer/{consumer_pid}` | Check transfer state (→ endpoint + token) |

See [`docker-compose/USAGE.md`](../docker-compose/USAGE.md) for a runnable
end-to-end example using these.
