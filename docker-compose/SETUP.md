# Demo setup (native DCP)

Two connectors, `party-a` and `party-b`, each running the built-in native DCP wallet
(issuer + holder + verifier + STS behind a single `did:web`). There is no external
wallet, verifier, or issuer service.

## Build the image

```sh
./build.sh                 # builds connector:latest (compiles inside the image)
```

## Start services

```sh
cd docker-compose
docker compose up -d
```

- `party-a` → http://localhost:13000
- `party-b` → http://localhost:23000

Each connector serves its DID document at `/.well-known/did.json`, derived from its
`participant_info.external_address`, e.g. `did:web:party-a-connector%3A3000`. The
document advertises the connector's `CredentialService` and `IssuerService`.

## Keys and trust

Each party's signing key lives in `[party-a|party-b]/connector/config.json` as
`private_key_pem` — the same key signs SI tokens, issued credentials, and DSP access
tokens, and its public JWK is published in the DID document. To generate a fresh key:

```sh
cargo run --bin keygen --features keygen
```

Paste the PEM into the respective `config.json` and restart the connector.

Trust is configured via `allowed_issuers`: the DIDs whose issued credentials a
verifier will accept. The demo uses the **bilateral** model — each party's
`allowed_issuers` lists both parties, so each connector trusts credentials issued by
either side.

## Seed identity credentials

Before a party can authenticate, it must hold an `identity_credential` issued by a
trusted issuer. Trigger native issuance so each party obtains a credential issued by
the counterparty:

```sh
# party-b obtains a credential issued by party-a
curl -s -X POST http://localhost:23000/api/credentials/v1/request \
  -H 'Content-Type: application/json' \
  -d '{"issuerDid":"did:web:party-a-connector%3A3000"}'

# party-a obtains a credential issued by party-b
curl -s -X POST http://localhost:13000/api/credentials/v1/request \
  -H 'Content-Type: application/json' \
  -d '{"issuerDid":"did:web:party-b-connector%3A3000"}'
```

The holder resolves the issuer's `IssuerService`, requests the credential, and the
issuer delivers it back to the holder's `CredentialService`, which stores it under
`[party-a|party-b]/connector/data/credentials` (files created `0600`).

Verify:

```sh
curl -s http://localhost:13000/api/credentials/v1/credentials | jq
curl -s http://localhost:23000/api/credentials/v1/credentials | jq
```

## Authentication

No manual step is required. When a connector needs a DSP access token from a peer, it
mints a Self-Issued ID Token and calls the peer's `POST /auth/token`; the peer pulls a
verifiable presentation from the requester's `CredentialService`, validates it against
`allowed_issuers`, and returns the access token. Catalog sync (driven by the
`federation` config) exercises this automatically — see `USAGE.md`.
