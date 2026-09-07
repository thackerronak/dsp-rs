# Native DCP authentication

This connector implements the
[Decentralized Claims Protocol](https://eclipse-dataspace-dcp.github.io/decentralized-claims-protocol/v1.0.1/)
natively. Each connector is its own holder and verifier — and, for self-issued
credentials, its own issuer — behind a single `did:web`. There is no external
wallet or verifier service.

Token exchange is a synchronous **pull**: no session, no polling. The verifier
reaches back into the requester's Credential Service to fetch a presentation.

`Authenticator` (`mod.rs`) owns the wallet and is a concrete type — there is one
implementation, so there is no backend trait and nothing downcasts in the request
path. The inbound verifier handler lives in `token.rs`; the DCP primitives it
builds on live in `../dcp/`.

```mermaid
sequenceDiagram
    participant A as Connector A (requester / holder)
    participant B as Connector B (verifier)

    rect rgba(255, 0, 43, 0.15)
    note over A,B: Authentication Phase (native DCP pull)
    activate A
    A->>A: build_si_token() — sign SI token, aud = B
    A->>B: POST /auth/token (Bearer SI token)
    activate B
    B->>B: validate SI token (signature via A's DID doc, aud, exp, jti replay)
    B->>B: resolve A's CredentialService from its DID document
    B->>A: POST /api/credentials/v1/presentations/query (Bearer B's SI token, scope)
    A-->>B: PresentationResponseMessage { JWT-VP }
    B->>B: validate VP + VC (issuer in allowed_issuers)
    B->>B: derive_access_token() — map credential to claims
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

## Where the SI token comes from

`Authenticator::get_token` mints the Self-Issued ID Token **in process** via
`build_si_token`. The Secure Token Service (`../dcp/sts.rs`) is *not* on this path
— nothing in the protocol calls it. It exists so an operator or an external client
can mint a token by hand, which is why it is opt-in (`dcp.sts_client_id` /
`dcp.sts_client_secret`, no defaults) and mounted at
`POST /api-internal/sts/token` rather than on a public path.

## Layering

```
connector  ->  auth  ->  dcp  ->  shared
```

`dcp` depends only on `shared` (`KeyPair`, DID-document types, did:web helpers)
and external crates — never on `connector` or `auth`. A test in `dcp/mod.rs`
enforces this, so lifting the wallet into its own crate or process stays a move
rather than a redesign.
