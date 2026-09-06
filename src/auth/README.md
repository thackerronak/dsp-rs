# Native DCP authentication protocol

The sequence diagram below illustrates the connector's built-in authentication — a
native implementation of the [Decentralized Claims Protocol](https://eclipse-dataspace-dcp.github.io/decentralized-claims-protocol/v1.0.1/).
Each connector is its own issuer, holder, and verifier behind a single `did:web`;
there is no external wallet or verifier service. Token exchange is a synchronous
**pull** — no session, no polling.

```mermaid
sequenceDiagram
    participant A as Connector A (requester / holder)
    participant B as Connector B (verifier)

    rect rgba(255, 0, 43, 0.15)
    note over A,B: Authentication Phase (native DCP pull)
    activate A
    A->>A: STS mints Self-Issued ID Token (aud = B)
    A->>B: POST /auth/token (Bearer SI token)
    activate B
    B->>B: validate SI token (signature via A's DID doc, aud, jti replay, exp)
    B->>A: POST /api/credentials/v1/presentations/query (Bearer B's SI token, scope)
    A-->>B: PresentationResponseMessage { JWT-VP }
    B->>B: validate VP + VC (issuer in allowed_issuers), derive_access_token()
    B-->>A: { access_token, token_type: Bearer, expires_in }
    deactivate B
    deactivate A
    end

    rect rgba(0, 255, 34, 0.4)
    note over A,B: Negotiation Phase
    activate A
    Note over A,B: Authorization: Bearer <<access_token>>
    A->>B: POST /api/2025/1/negotiations/request
    activate B
    B->>A:
    deactivate B
    deactivate A
    end
```
