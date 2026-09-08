# Demo Setup

Two connectors and one issuer with dual-mode configuration (localhost or HTTPS via NGrok).

Each connector runs a **native wallet** — its own holder and verifier behind a
single `did:web`. It serves its own `/.well-known/did.json` and stores its own
credentials, so there is no wallet or verifier service to configure. The only
external identity component is the **walt.id issuer**, which is the point: an issuer
should be a third party, not the connector vouching for itself.

## Architecture

### Services

| Service | Port | Role | DID |
|---|---|---|---|
| `issuer` | 7002 | walt.id issuer — signs credentials | `did:web:issuer-did-server` |
| `issuer-did-server` | 17002 | serves the issuer's DID document | — |
| `party-a-connector` | 13000 | connector (localhost) | `did:web:party-a-connector%3A3000` |
| `party-b-connector` | 23000 | connector (localhost) | `did:web:party-b-connector%3A3000` |

With `ENFORCE_HTTPS=true`, connectors expose via HTTPS ngrok tunnels with their ngrok domains as DIDs.

### Modes

**Development (default)** — `ENFORCE_HTTPS=false`
- Localhost HTTP only
- No external dependencies
- Fast local testing

**Production** — `ENFORCE_HTTPS=true`
- HTTPS via NGrok tunnels
- Requires NGrok auth tokens and reserved domains
- Real TLS encryption

## Quick Start

### 1. Generate Configuration

```sh
cd docker-compose
./setup-configs.sh
```

This will:
- Create `.env` from `.env.sample` (if missing)
- Generate connector `config.json` files
- Default: localhost HTTP, ready to start

### 2. Start Services

**Localhost (default):**
```sh
docker-compose up -d
```

**NGrok HTTPS (optional):**
```sh
# Edit .env first: set ENFORCE_HTTPS=true and add NGrok credentials
vi .env
./setup-configs.sh
docker-compose --profile ngrok up -d
```

### 3. Verify Startup

```sh
docker-compose ps
docker-compose logs -f party-a-connector
```

## Configuration Files

- `.env` — Environment variables (auto-created from `.env.sample`, never committed)
- `.env.sample` — Template with all available options
- `party-{a,b}/connector/config.json.template` — Config templates (with variable substitution)
- `party-{a,b}/connector/config.json` — Generated configs (never committed, created by setup script)

## Seeding Credentials

### 1. Seed each connector with a credential

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

### 2. Check the handshake

Optional — connectors do this themselves before any DSP call. Mint a Self-Issued ID
Token as party B addressed to party A, then exchange it:

```sh
SI=$(curl -s -X POST http://localhost:23000/api-internal/sts/token \
  -H 'content-type: application/x-www-form-urlencoded' \
  -d 'grant_type=client_credentials&client_id=dsp-client&client_secret=dsp-secret&audience=did%3Aweb%3Aparty-a-connector%3A3000' \
  | jq -r .access_token)

curl -s -X POST http://localhost:13000/auth/token -H "Authorization: Bearer $SI" | jq
```

A `200` with an `access_token` means the whole chain worked: SI token validated,
presentation pulled from party B's Credential Service, VP and VC verified, claims
mapped.

A `401` means one of those failed. Run the connectors with `RUST_LOG=debug` — the
verifier logs which step it was. The usual causes are no credential seeded or an issuer 
missing from `allowed_issuers`.

## Configuration

Connector configs are **generated** from templates using environment variables in `.env`:

```
setup-configs.sh:
  .env (ENFORCE_HTTPS, keys, domains)
    ↓
  config.json.template + environment substitution
    ↓
  config.json (generated)
```

### Config File Structure

`[party-a|party-b]/connector/config.json`:

| Key | Source | Meaning |
|---|---|---|
| `participant_info.external_address` | `${PARTY_*_EXTERNAL_ADDRESS}` from .env | HTTP/HTTPS endpoint where this connector is reachable |
| `participant_info.id` | hardcoded | Participant name |
| `private_key_pem` | `${PARTY_*_PRIVATE_KEY_PEM}` from .env | Signs DSP access and transfer tokens. Not published in DID. |
| `wallet.private_key_pem` | `${PARTY_*_WALLET_PRIVATE_KEY_PEM}` from .env | Signs SI tokens, presentations, and credentials. Published in DID as `#keys-1`. |
| `federation.<name>.did` | `${PARTY_*_DID_HOST}` from .env | Peer's DID (resolved from their DID document) |
| `issuer_url` | hardcoded | walt.id issuer endpoint |
| `allowed_issuers` | hardcoded | List of trusted issuers |
| `wallet.sts_client_id` / `sts_client_secret` | from .env | STS credentials for token exchange |

### Environment-Driven Configuration

All sensitive and environment-specific values come from `.env`:

```bash
# From .env
ENFORCE_HTTPS=false                    # localhost or HTTPS
PARTY_A_NGROK_DOMAIN=...             # (for HTTPS mode)
PARTY_A_PRIVATE_KEY_PEM=...           # EC P-256 key
PARTY_A_WALLET_PRIVATE_KEY_PEM=...    # EC P-256 key
```

Generated addresses (localhost default):
- `http://party-a-connector:3000` → DID: `did:web:party-a-connector%3A3000`
- `http://party-b-connector:3000` → DID: `did:web:party-b-connector%3A3000`

(With ENFORCE_HTTPS=true, uses HTTPS ngrok domains instead — see `.env.sample`)

### Generating Fresh Keys

To generate new EC P-256 keypairs:

```sh
cargo run --bin keygen --features keygen
```

Output: PKCS#8 PEM (newlines escaped for .env) + JWK. Copy the PEM into `.env`:

```bash
PARTY_A_PRIVATE_KEY_PEM="-----BEGIN PRIVATE KEY-----\r\n..."
PARTY_A_WALLET_PRIVATE_KEY_PEM="-----BEGIN PRIVATE KEY-----\r\n..."
```

Then regenerate configs:
```sh
./setup-configs.sh
```

**Note:** Changing `wallet.private_key_pem` invalidates credentials issued to that key.
Reseed credentials after rotation.

## Alternative: Self-Issued Credentials

A connector is also a DCP issuer, so parties can issue to each other with no external issuer.
Trust is configuration rather than a third party, so this is a fallback.

Edit connector config (or templates before regenerating) to add party DIDs to `allowed_issuers`:

```json
"allowed_issuers": [
  "did:web:issuer-did-server",
  "did:web:party-a-connector%3A3000",
  "did:web:party-b-connector%3A3000"
]
```

Request a credential:
```sh
curl -X POST http://localhost:23000/api/credentials/v1/request \
  -H 'content-type: application/json' \
  -d '{"issuerDid":"did:web:party-a-connector%3A3000"}'
```

Returns `202` — delivery is asynchronous push. Poll until credential lands:
```sh
curl -s http://localhost:23000/api/credentials/v1/credentials | jq
```

An issuer can also push an offer:
```sh
curl -X POST http://localhost:13000/api/issuance/v1/offer \
  -H 'content-type: application/json' \
  -d '{"holderDid":"did:web:party-b-connector%3A3000"}'
```

---

For HTTPS testing with NGrok, configure `ENFORCE_HTTPS=true` in `.env`, then regenerate configs with `./setup-configs.sh`.

---

Next: [negotiating a contract and accessing a dataset](USAGE.md).
