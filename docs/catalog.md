# Stage 1 — Catalog (browse the shop) 🛒

_Part of the [DSP guide](README.md) · Stage 1 of 4_

Read-only browsing. Nothing is reserved, nothing signed, no data moves.
Defined in `src/catalog/mod.rs`.

| Endpoint | Meaning |
|----------|---------|
| `POST /api/2025/1/catalog/request` | "Show me your whole catalogue" |
| `GET /api/2025/1/catalog/datasets/{id}` | "Show me the detail page for this one product" |

## `POST /catalog/request`

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

## `GET /catalog/datasets/{id}`

If you already know the barcode and want just that one item's detail page — returns
the single dataset, or `404 dataset not found` (`dataset_request`,
`src/catalog/mod.rs`).

## Key mental model

- Catalog = **read-only browsing**. Nothing is reserved, signed, or moved.
- In this repo the consumer usually mirrors the producer's catalog ahead of time
  via **catalog sync** and negotiates against a local cache — see
  [Producer & Discovery](producer-and-discovery.md).

> 🚗 **Catena-X:** a producer publishes an **asset + policy + contract-definition**
> in its EDC; a consumer runs this exact catalog request against it.

---

**Prev:** [← Why DSP?](why-dsp.md) · **Next:** [Producer & Discovery →](producer-and-discovery.md) · [Index](README.md)
