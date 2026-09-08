# Demo setup

Two connectors and one issuer.

Each connector runs a **native DCP wallet** — its own holder and verifier behind a
single `did:web`. It serves its own `/.well-known/did.json` and stores its own
credentials, so there is no wallet or verifier service to configure. The only
external identity component is the **walt.id issuer**, which is the point: an issuer
should be a third party, not the connector vouching for itself.

| Service | Port | Role |
|---|---|---|
| `issuer` | 7002 | walt.id issuer — signs credentials |
| `issuer-did-server` | 17002 | serves the issuer's DID document |
| `party-a-connector` | 13000 | connector (`did:web:party-a-connector%3A3000`) |
| `party-b-connector` | 23000 | connector (`did:web:party-b-connector%3A3000`) |

## 1. Build and start

```sh
./build.sh                                          # connector:latest
docker compose -f docker-compose/docker-compose.yml up
```

## 2. Seed each connector with a credential

The connector redeems an OID4VCI offer itself, signing the holder proof with its own
key — nothing is copied into an external wallet.

```sh
# ask the issuer for a pre-authorized offer
OFFER=$(curl -s -X POST 'http://localhost:7002/issuer2/credential-offers' \
  -H 'content-type: application/json' \
  -d '{"profileId":"identityCredentialJwtVc","authMethod":"PRE_AUTHORIZED"}' \
  | jq -r '.credentialOffer')

# hand it to party B, which runs the flow and stores the credential
curl -s -X POST http://localhost:23000/api-internal/credentials/redeem \
  -H 'content-type: application/json' \
  -d "{\"offerUrl\":\"$OFFER\"}" | jq
```

Repeat with a fresh offer against `http://localhost:13000` so party A can authenticate
to party B. Confirm what a connector holds:

```sh
curl -s http://localhost:23000/api/credentials/v1/credentials | jq
```

Use the `identityCredentialJwtVc` profile. The bundled `identityCredentialSdJwt`
profile issues an IETF SD-JWT VC, which this connector's verifier cannot read.

## 3. Check the handshake

Optional — connectors do this themselves before any DSP call. Mint a Self-Issued ID
Token as party B addressed to party A, then exchange it:

```sh
SI=$(curl -s -X POST http://localhost:23000/api-internal/sts/token \
  -H 'content-type: application/x-www-form-urlencoded' \
  -d 'grant_type=client_credentials&client_id=dsp-client&client_secret=dsp-secret&audience=did%3Aweb%3Aparty-a-connector%253A3000' \
  | jq -r .access_token)

curl -s -X POST http://localhost:13000/auth/token -H "Authorization: Bearer $SI" | jq
```

A `200` with an `access_token` means the whole chain worked: SI token validated,
presentation pulled from party B's Credential Service, VP and VC verified, claims
mapped.

A `401` means one of those failed. Run the connectors with `RUST_LOG=debug` — the
verifier logs which step it was. The usual causes are no credential seeded (step 2)
or an issuer missing from `allowed_issuers`.

## Configuration

`[party-a|party-b]/connector/config.json`:

| Key | Meaning |
|---|---|
| `private_key_pem` | Signs SI tokens, presentations and access tokens. Its public half is published in the connector's DID document. |
| `issuer_url` | The walt.id issuer. An offer naming a different issuer is refused. |
| `allowed_issuers` | Whose credentials the verifier accepts. `did:web:issuer-did-server` is the walt.id issuer. |
| `dcp.sts_client_id` / `sts_client_secret` | Enable the STS used in step 3. Omit both and it is not mounted — there are no default credentials. |

To generate fresh keys:

```sh
cargo run --bin keygen --features keygen
```

It prints a PKCS#8 PEM (newlines escaped, ready to paste) then the matching JWK. Only
the PEM is needed; the public half is derived from it.

## Alternative: self-issued credentials

A connector is also a DCP issuer, so the parties can issue to each other with no
external issuer at all. Trust here is configuration rather than a third party, so it
is a fallback rather than the demo path — add the party DIDs to `allowed_issuers` in
**both** configs first:

```json
"allowed_issuers": [
  "did:web:issuer-did-server",
  "did:web:party-a-connector%3A3000",
  "did:web:party-b-connector%3A3000"
]
```

```sh
curl -X POST http://localhost:23000/api/credentials/v1/request \
  -H 'content-type: application/json' \
  -d '{"issuerDid":"did:web:party-a-connector%3A3000"}'
```

This returns `202`: unlike the OID4VCI redeem, delivery is an asynchronous push, so
poll `GET /api/credentials/v1/credentials` until it lands. An issuer can also push an
offer with `POST /api/issuance/v1/offer -d '{"holderDid":"…"}'`, which triggers the
same exchange.

---

Next: [negotiating a contract and accessing a dataset](USAGE.md).
