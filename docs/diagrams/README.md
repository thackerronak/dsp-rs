# DSP Sequence Diagrams

## Dataspace Protocol (DSP 2025-1) end-to-end flow

The diagram below renders directly in the GitHub UI (Mermaid). A more detailed
PlantUML version is kept alongside it in
[`dsp-protocol-sequence.puml`](dsp-protocol-sequence.puml). Both mirror the demo
in [`docker-compose/USAGE.md`](../../docker-compose/USAGE.md).

```mermaid
sequenceDiagram
    autonumber
    actor User as Data Consumer (internal API)
    participant Consumer as Consumer Connector<br/>did:web:party-b-connector%3A3000
    participant Wallet as Consumer Wallet<br/>(SSI / OpenID4VP)
    participant Provider as Provider Connector<br/>did:web:party-a-connector%3A3000

    Note over Consumer,Provider: All DSP messages use context https://w3id.org/dspace/2025/1/context.jsonld<br/>on base path /api/2025/1. Negotiation & Transfer are asynchronous:<br/>the receiver ACKs immediately, then pushes state changes to callbackAddress.

    rect rgb(235, 245, 251)
    Note over Consumer,Provider: STEP 0 — Catalog discovery (optional)
    User->>Consumer: POST /api-internal/negotiate
    Consumer->>Provider: POST /api/2025/1/catalog/request (CatalogRequestMessage)
    Provider-->>Consumer: 200 OK — dcat:Catalog (datasets, policies, distribution)
    end

    rect rgb(253, 237, 236)
    Note over Consumer,Provider: STEP 1 — Self-issued token handshake (DCP)
    Consumer->>Provider: POST /auth/verify_me { did_web }
    Provider-->>Consumer: 200 OK — Session { session_id, openid4vp_url }
    Consumer->>Wallet: POST /wallet/{id}/credentials/present
    Wallet-->>Provider: OpenID4VP — submit Verifiable Presentation
    loop Poll until status == valid (max ~10s, every 3s)
        Consumer->>Provider: GET /auth/status/{session_id}
        Provider-->>Consumer: 200 OK — Status (pending | valid + access_token)
    end
    Note over Consumer: Consumer now holds a Bearer si_token (ES256) for the Provider
    end

    rect rgb(234, 250, 241)
    Note over Consumer,Provider: STEP 2 — Contract negotiation (async callbacks)
    Consumer->>Provider: POST /negotiations/request<br/>ContractRequestMessage (offer + callbackAddress) [Bearer]
    Provider->>Consumer: GET /.well-known/did.json (resolve DID, verify ES256 token)
    Consumer-->>Provider: 200 OK — Consumer DID Document
    Provider-->>Consumer: 201 Created — ContractNegotiation (REQUESTED)
    alt Provider sends a (counter) offer first
        Provider->>Consumer: POST /negotiations/{consumerPid}/offers (ContractOfferMessage)
        Consumer-->>Provider: 200 OK (OFFERED)
        Consumer->>Provider: POST /negotiations/{providerPid}/request (accept offer)
        Provider-->>Consumer: 200 OK (ACCEPTED)
    else Provider agrees to the original offer
        Note over Provider: Evaluate offer against policy engine
    end
    Provider->>Consumer: POST /negotiations/{consumerPid}/agreement (ContractAgreementMessage)
    Consumer-->>Provider: 200 OK (AGREED)
    Consumer->>Provider: POST /negotiations/{providerPid}/agreement/verification
    Provider-->>Consumer: 200 OK (VERIFIED)
    Provider->>Consumer: POST /negotiations/{consumerPid}/events (eventType FINALIZED)
    Consumer-->>Provider: 200 OK (FINALIZED)
    Note over Consumer: Consumer stores the finalized Agreement
    end

    rect rgb(254, 249, 231)
    Note over Consumer,Provider: STEP 3 — Transfer process (async callbacks)
    Consumer->>Provider: POST /transfers/request<br/>TransferRequestMessage (agreementId, format, callbackAddress) [Bearer]
    Provider-->>Consumer: 201 Created — TransferProcess (REQUESTED)
    Provider->>Consumer: POST /transfers/{consumerPid}/start<br/>TransferStartMessage (DataAddress: pull endpoint + transfer_token)
    Consumer-->>Provider: 200 OK (STARTED)
    opt Consumer pulls the data (HttpData-PULL)
        Consumer->>Provider: GET/POST /pull/... [Authorization: transfer_token]
        Provider-->>Consumer: 200 OK — dataset payload (via reverse proxy)
    end
    Consumer->>Provider: POST /transfers/{providerPid}/completion (TransferCompletionMessage)
    Provider-->>Consumer: 200 OK (COMPLETED)
    end

    Note over Consumer,Provider: Either party may also send TransferSuspension/Termination<br/>or ContractNegotiationTermination at the matching endpoints.
```

### Flow summary

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

### Rendering the PlantUML source

```sh
# PNG
plantuml docs/diagrams/dsp-protocol-sequence.puml

# SVG
plantuml -tsvg docs/diagrams/dsp-protocol-sequence.puml
```

Or paste the file into the [PlantUML web server](https://www.plantuml.com/plantuml).
