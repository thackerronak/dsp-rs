# DSP Sequence Diagrams

## Dataspace Protocol (DSP 2025-1) end-to-end flow

[`dsp-protocol-sequence.puml`](dsp-protocol-sequence.puml) is a PlantUML sequence
diagram of the full consumer/provider exchange implemented by this connector,
mirroring the demo in [`docker-compose/USAGE.md`](../../docker-compose/USAGE.md):

- **Step 0 — Catalog discovery**: `POST /api/2025/1/catalog/request` returns the
  provider's `dcat:Catalog` (see `src/catalog/mod.rs`).
- **Step 1 — Self-issued token acquisition**: the DCP handshake
  (`/auth/verify_me` → OpenID4VP presentation via the wallet → poll
  `/auth/status/{session_id}`) yields the ES256 Bearer token that protects every
  DSP call. Incoming tokens are verified by resolving the caller's
  `did:web` document at `/.well-known/did.json` (see `src/auth/`).
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

### Rendering

```sh
# PNG
plantuml docs/diagrams/dsp-protocol-sequence.puml

# SVG
plantuml -tsvg docs/diagrams/dsp-protocol-sequence.puml
```

Or paste the file into the [PlantUML web server](https://www.plantuml.com/plantuml).
