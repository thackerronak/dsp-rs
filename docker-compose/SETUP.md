# Demo setup

Two connectors, each running a **native DCP wallet** — its own holder, verifier and
issuer behind a single `did:web`. There is no external wallet or verifier service to
configure: the connector serves its own `/.well-known/did.json` and stores its own
credentials.

> **Note:** `docker-compose.yml` still starts the walt.id `wallet` and `verifier`
> containers. The connector no longer talks to them and they can be ignored; removing
> them from the compose file is a separate change.

## 1. Start services

```sh
docker compose -f docker-compose/docker-compose.yml up
```

Party A is published on `localhost:13000`, party B on `localhost:23000` (both are
`:3000` inside the network).

## 2. Generate a keypair per participant

Only needed if you want fresh keys — the demo configs ship with working ones.

```sh
cargo run --bin keygen --features keygen
```

It prints a PKCS#8 PEM (with escaped newlines, ready to paste) followed by the
matching JWK. Put the PEM in `private_key_pem` in
`[party-a|party-b]/connector/config.json`. The public half is published in the
connector's DID document automatically — there is no wallet to import it into.

## 3. Configure trust

The verifier only accepts a credential whose issuer is listed in `allowed_issuers`.
For the native path — where each connector issues to the other — list both party
DIDs in **both** configs:

```json
"allowed_issuers": [
  "did:web:party-a-connector%3A3000",
  "did:web:party-b-connector%3A3000"
]
```

Optionally enable the Secure Token Service, so you can mint an SI token by hand to
drive `/auth/token` yourself. It is off unless both values are set — there are no
default credentials:

```json
"dcp": {
  "sts_client_id": "dsp-client",
  "sts_client_secret": "dsp-secret"
}
```

Other `dcp` keys (`credential_store_path`, `credential_service_path`,
`issuance_service_path`) have sensible defaults. Restart after editing.

## 4. Seed a credential

Each party needs a credential in its store before it can prove anything. Ask party
B's holder to request one from party A's issuer:

```sh
curl -X POST http://localhost:23000/api/credentials/v1/request \
  -H 'content-type: application/json' \
  -d '{"issuerDid":"did:web:party-a-connector%3A3000"}'
```

This returns `202`: the issuer mints the credential and **pushes** it back to the
holder asynchronously. Poll until it lands:

```sh
curl -s http://localhost:23000/api/credentials/v1/credentials | jq
```

Repeat in the other direction so party A can authenticate to party B:

```sh
curl -X POST http://localhost:13000/api/credentials/v1/request \
  -H 'content-type: application/json' \
  -d '{"issuerDid":"did:web:party-b-connector%3A3000"}'
```

### From an external (walt.id) issuer

If you would rather have a real third party issue the credential, the connector can
redeem an OID4VCI offer itself:

```sh
# 1. ask the walt.id issuer for a pre-authorized offer
OFFER=$(curl -s -X POST 'http://localhost:7002/issuer2/credential-offers' \
  -H 'content-type: application/json' \
  -d '{"profileId":"identityCredentialJwtVc","authMethod":"PRE_AUTHORIZED"}' | jq -r '.credentialOffer')

# 2. hand it to the connector, which runs the flow and stores the credential
curl -s -X POST http://localhost:23000/api-internal/credentials/redeem \
  -H 'content-type: application/json' \
  -d "{\"offerUrl\":\"$OFFER\"}" | jq
```

The connector signs the OID4VCI holder proof with its own key — unlike the previous
setup, no private key is copied into an external wallet.

> **Not runnable yet:** the bundled issuer config only advertises a `dc+sd-jwt`
> credential, which this connector's verifier cannot read. Add a `jwt_vc_json`
> credential configuration and matching profile under `docker-compose/issuer/config/`
> first. With `allowed_issuers` set to `["did:web:issuer-did-server"]`, this becomes
> the primary path and the DCP issuance above becomes the self-issued fallback.

An issuer can also push an offer instead, which triggers the same exchange:

```sh
curl -X POST http://localhost:13000/api/issuance/v1/offer \
  -H 'content-type: application/json' \
  -d '{"holderDid":"did:web:party-b-connector%3A3000"}'
```

## 5. Check the token exchange

With STS enabled you can run the handshake by hand — mint an SI token as party B
addressed to party A, then exchange it:

```sh
SI=$(curl -s -X POST http://localhost:23000/api-internal/sts/token \
  -H 'content-type: application/x-www-form-urlencoded' \
  -d 'grant_type=client_credentials&client_id=dsp-client&client_secret=dsp-secret&audience=did%3Aweb%3Aparty-a-connector%253A3000' \
  | jq -r .access_token)

curl -s -X POST http://localhost:13000/auth/token -H "Authorization: Bearer $SI" | jq
```

A `200` with an `access_token` means the full chain worked: SI token validated,
presentation pulled from party B's Credential Service, VP and VC verified, claims
mapped. A `401` usually means the credential's issuer is not in `allowed_issuers`
(step 3) or no credential was seeded (step 4).

Connectors do all of this for themselves before any DSP call, so this step is only
for verifying the setup.

---

Next: [negotiating a contract and accessing a dataset](USAGE.md).
