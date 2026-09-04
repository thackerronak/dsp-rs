# Producer & Discovery — publishing data and finding it

_Part of the [DSP guide](README.md) · Companion to Stage 1_

The rest of the guide is from the *consumer's* point of view. This page is the
other half: how a **producer** (AutoParts) offers data, and how a **consumer**
(InsureCo) finds out it exists.

## Publishing: you describe datasets, the catalog is assembled on the fly

There is **no "create catalog" endpoint.** A producer just **publishes dataset
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
  an internal API, a file server). The `/pull` proxy forwards here in
  [Stage 4](transfer.md) (`get_dataset_target`, `src/store/file_store.rs`).
  **Buyers never see this URL.**

When a consumer calls `POST /catalog/request`, the handler (`catalog_request`,
`src/catalog/mod.rs`):
1. reads **every** file in `data/datasets/` (`get_datasets`, `src/store/file_store.rs`);
2. **rewrites** each distribution's endpoint to the connector's *own* address (the
   pickup counter is the connector, not the raw backend);
3. wraps them in a fresh `RootCatalog` and returns it.

> The datasets on disk are the **source of truth**; the "catalog" is a **live view**
> assembled at request time. Add a file → it's instantly in the catalog.
> (`file_store` is one implementation of the `Store` trait; a real deployment could
> use a database without changing any endpoint.)

## Discovery: how a consumer knows a producer exists

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
   version (`/.well-known/dspace-version`), gets a token
   ([Stage 2](auth-and-identity.md)), calls the producer's `POST /catalog/request`,
   and **caches** the datasets locally as *federated datasets*
   (`save_federated_dataset` → `data/datasets/federated/<name>/`).
3. **Negotiate against the cache.** `POST /api-internal/negotiate {"name":"party-a"}`
   looks up the cached federated dataset and starts [Stage 3](negotiation.md).

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

## Discovery is a layer *above* DSP

| Approach | How discovery works |
|----------|--------------------|
| **This repo** | Static `federation` config + periodic catalog sync (you must know the address) |
| **Catena-X** | Central **Discovery Finder** + **BPN Discovery Service** resolve a partner's **BPN → connector URL**, plus a **Digital Twin Registry** and federated-catalog crawlers |
| **Gaia-X / IDS** | A **Federated Catalogue / Broker** where participants register self-descriptions and others query centrally |

> **DSP** = the *bilateral* catalog protocol ("ask a **known** producer for its
> list"). **Discovery** = a *separate* concern ("find **which** producers exist and
> **where**"). Here it's config; in Catena-X it's a central Discovery Service.

---

**Prev:** [← Stage 1 — Catalog](catalog.md) · **Next:** [Stage 2 — Auth & Identity →](auth-and-identity.md) · [Index](README.md)
