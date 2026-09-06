# DSP Sequence Diagrams

## Dataspace Protocol (DSP 2025-1) end-to-end flow

The diagram below is generated on the fly by the PlantUML server from
[`dsp-protocol-sequence.puml`](dsp-protocol-sequence.puml) on the `docs` branch —
no local tooling needed. It mirrors the demo in
[`docker-compose/USAGE.md`](../../docker-compose/USAGE.md).

> New to these endpoints? Read the step-by-step, example-driven guide starting at
> the [docs index](../README.md).

![DSP 2025-1 sequence diagram (PlantUML)](https://www.plantuml.com/plantuml/proxy?cache=no&fmt=svg&src=https://raw.githubusercontent.com/thackerronak/dsp-rs/docs/docs/diagrams/dsp-protocol-sequence.puml)

- **Live link (SVG):** <https://www.plantuml.com/plantuml/proxy?cache=no&fmt=svg&src=https://raw.githubusercontent.com/thackerronak/dsp-rs/docs/docs/diagrams/dsp-protocol-sequence.puml>
- **Live link (PNG):** <https://www.plantuml.com/plantuml/proxy?cache=no&fmt=png&src=https://raw.githubusercontent.com/thackerronak/dsp-rs/docs/docs/diagrams/dsp-protocol-sequence.puml>

> The `proxy?src=<raw URL>` form makes `plantuml.com` fetch and render the file
> itself. It reads the raw file from the `docs` branch — change the branch segment
> in the URL (`.../dsp-rs/<branch>/docs/diagrams/...`) to render another branch.

### Flow summary

- **Setup (Provider)** — the data owner publishes a dataset descriptor file
  (`data/datasets/<id>.json`: public `dataset` + hidden `remoteAddress` backend);
  the catalog is assembled per request (see `src/catalog/`).
- **Step 0 — Catalog discovery**: `POST /api/2025/1/catalog/request` returns the
  provider's `dcat:Catalog` (see `src/catalog/mod.rs`).
- **Step 1 — Access token acquisition**: the native DCP pull
  (`POST /auth/token` with a Self-Issued ID Token → the verifier pulls a JWT-VP
  from the caller's `/api/credentials/v1/presentations/query` → validates it) yields
  the ES256 Bearer access token that protects every DSP call — synchronous, no
  polling. Incoming access tokens are self-signed by the minting connector and
  verified by its `AuthClaims` extractor (see `src/auth/` and `src/dcp/`).
- **Step 2 — Contract negotiation**: asynchronous, callback-based state machine
  (`REQUESTED → OFFERED/ACCEPTED → AGREED → VERIFIED → FINALIZED`) over
  `/api/2025/1/negotiations/*` (see `src/negotiation/`).
- **Step 3 — Transfer process**: `TransferRequestMessage` → provider
  `TransferStartMessage` carrying the `DataAddress` (pull endpoint + transfer
  token) → data pull via the `/pull` reverse proxy → `TransferCompletionMessage`
  (see `src/transfer/` and `src/reverse_proxy/`).

Because negotiation and transfer are asynchronous, the receiver ACKs immediately
(`201`/`200`) and later pushes each state change to the sender's
`callbackAddress`.

### Rendering the PlantUML source locally

```sh
# PNG
plantuml docs/diagrams/dsp-protocol-sequence.puml

# SVG
plantuml -tsvg docs/diagrams/dsp-protocol-sequence.puml
```

Or paste the file into the [PlantUML web server](https://www.plantuml.com/plantuml).
