#!/bin/sh
# Brings party C from "runtimes are up" to "ready to trade with party A":
#   1. a participant context in IdentityHub, which publishes party C's DID document
#   2. an identity credential, requested from party A's DCP issuer
#   3. a registered data plane, plus one asset to offer party A
#
# Idempotent: every step treats "already exists" as success, so re-running is safe.
set -eu

: "${KEYCLOAK_URL:?}" "${KEYCLOAK_CLIENT_ID:?}" "${KEYCLOAK_CLIENT_SECRET:?}"
: "${IH_IDENTITY_URL:?}" "${IH_CREDENTIAL_SERVICE_URL:?}"
: "${CP_MANAGEMENT_URL:?}" "${CP_MANAGEMENT_KEY:?}" "${CP_PROTOCOL_URL:?}"
: "${DP_SIGNALING_URL:?}" "${CP_SIGNALING_URL:?}" "${DP_DATAFLOW_URL:?}"
: "${PARTY_C_CONTEXT_ID:?}" "${PARTY_C_DID:?}"
: "${PARTY_A_DID:?}"
: "${ASSET_ID:?}"

step() { printf '\n=== %s\n' "$1"; }

# Sends a request and prints the body; fails the script on anything but 2xx or 409.
call() {
  method="$1"; url="$2"; auth="$3"; body="${4:-}"
  if [ -n "$body" ]; then
    response=$(curl -sS -w '\n%{http_code}' -X "$method" "$url" \
      -H "$auth" -H 'Content-Type: application/json' -d "$body")
  else
    response=$(curl -sS -w '\n%{http_code}' -X "$method" "$url" -H "$auth")
  fi
  status=$(printf '%s' "$response" | tail -n1)
  CALL_BODY=$(printf '%s' "$response" | sed '$d')
  case "$status" in
    2*) return 0 ;;
    409) echo "  already exists (409), skipping" ; return 0 ;;
    *) echo "  HTTP $status"; echo "  $CALL_BODY"; return 1 ;;
  esac
}

# Like call(), but never fails the script: sets CALL_STATUS for the caller to branch on.
try_call() {
  method="$1"; url="$2"; auth="$3"; body="${4:-}"
  if [ -n "$body" ]; then
    response=$(curl -sS -w '\n%{http_code}' -X "$method" "$url" \
      -H "$auth" -H 'Content-Type: application/json' -d "$body" || printf '\n000')
  else
    response=$(curl -sS -w '\n%{http_code}' -X "$method" "$url" -H "$auth" || printf '\n000')
  fi
  CALL_STATUS=$(printf '%s' "$response" | tail -n1)
  CALL_BODY=$(printf '%s' "$response" | sed '$d')
}

wait_for() {
  name="$1"; url="$2"
  printf 'waiting for %s' "$name"
  i=0
  until curl -sf -o /dev/null "$url"; do
    i=$((i + 1))
    [ "$i" -gt 120 ] && { echo " timed out on $url"; exit 1; }
    printf '.'; sleep 2
  done
  echo ' ok'
}

step "0. waiting for dependencies"
wait_for "party C IdentityHub" "http://party-c-identityhub:7080/api/check/health"
wait_for "party C control plane" "http://party-c-controlplane:8080/api/check/health"
wait_for "party C data plane" "http://party-c-dataplane:8080/api/check/health"
wait_for "keycloak" "$KEYCLOAK_URL/realms/mvd/.well-known/openid-configuration"

step "1. Keycloak token for the Identity API"
TOKEN=$(curl -sS -X POST "$KEYCLOAK_URL/realms/mvd/protocol/openid-connect/token" \
  -d grant_type=client_credentials \
  -d "client_id=$KEYCLOAK_CLIENT_ID" \
  -d "client_secret=$KEYCLOAK_CLIENT_SECRET" \
  -d 'scope=identity-api:admin' | jq -r '.access_token')
[ "$TOKEN" != "null" ] && [ -n "$TOKEN" ] || { echo "no token from keycloak"; exit 1; }
IH_AUTH="Authorization: Bearer $TOKEN"
echo "  ok"

step "2. participant context (publishes $PARTY_C_DID)"
# A context created without a key pair is useless — it publishes a DID document with no
# verification method, which no peer can verify against. Clear such a leftover so a
# re-run recovers instead of silently keeping it.
try_call GET "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID" "$IH_AUTH"
if [ "$CALL_STATUS" = "200" ]; then
  try_call GET "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID/keypairs" "$IH_AUTH"
  key_count=$(printf '%s' "$CALL_BODY" | jq -r 'length // 0' 2>/dev/null || echo 0)
  if [ "$key_count" = "0" ]; then
    echo "  context exists with no key pair, recreating"
    try_call DELETE "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID" "$IH_AUTH"
  else
    echo "  context already has $key_count key pair(s)"
  fi
fi

# secp256r1 is P-256 under its JCE name, which is what the key generator expects. It has
# to be a P-256 key: party A validates every signature with ES256.
call POST "$IH_IDENTITY_URL/v1/participants" "$IH_AUTH" "$(cat <<JSON
{
  "participantContextId": "$PARTY_C_CONTEXT_ID",
  "did": "$PARTY_C_DID",
  "active": true,
  "key": {
    "keyId": "$PARTY_C_DID#key-1",
    "privateKeyAlias": "$PARTY_C_CONTEXT_ID-privatekey",
    "usage": ["sign_presentation", "sign_token"],
    "keyGeneratorParams": { "algorithm": "EC", "curve": "secp256r1" }
  },
  "serviceEndpoints": [
    {
      "id": "credential-service",
      "type": "CredentialService",
      "serviceEndpoint": "$IH_CREDENTIAL_SERVICE_URL"
    },
    {
      "id": "data-service",
      "type": "DataService",
      "serviceEndpoint": "$CP_PROTOCOL_URL/.well-known/dspace-version"
    },
    {
      "id": "protocol-endpoint",
      "type": "ProtocolEndpoint",
      "serviceEndpoint": "$CP_PROTOCOL_URL/2025-1"
    }
  ]
}
JSON
)"

# Creating an active context publishes the DID document; publish again in case the
# context survived from an earlier run in an unpublished state.
try_call POST "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID/dids/publish" "$IH_AUTH" '{}'
try_call GET "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID/keypairs" "$IH_AUTH"
echo "  key pairs: $(printf '%s' "$CALL_BODY" | jq -r 'length // 0' 2>/dev/null || echo '?')"

step "3. identity credential from party A's DCP issuer"
# Requesting again would just add a second copy of the same credential.
try_call GET "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID/credentials" "$IH_AUTH"
already_held=$(printf '%s' "$CALL_BODY" | jq --arg issuer "$PARTY_A_DID" '
  [.[] | select(.verifiableCredential.credential.issuer.id == $issuer)] | length' 2>/dev/null || echo 0)

if [ "${already_held:-0}" -gt 0 ]; then
  echo "  already holding $already_held credential(s) from $PARTY_A_DID, skipping"
else
HOLDER_PID=$(cat /proc/sys/kernel/random/uuid)
if call POST "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID/credentials/request" \
  "$IH_AUTH" "$(cat <<JSON
{
  "issuerDid": "$PARTY_A_DID",
  "holderPid": "$HOLDER_PID",
  "credentials": [
    { "format": "vc11-sl2021/jwt", "type": "identity_credential", "id": "identity_credential" }
  ]
}
JSON
)"; then
  i=0
  while [ "$i" -lt 30 ]; do
    call GET "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID/credentials/request/$HOLDER_PID" "$IH_AUTH" || true
    state=$(printf '%s' "$CALL_BODY" | jq -r '.status // .state // empty' 2>/dev/null || true)
    echo "  request status: ${state:-unknown}"
    [ "$state" = "ISSUED" ] && break
    i=$((i + 1)); sleep 2
  done
fi
fi

call GET "$IH_IDENTITY_URL/v1/participants/$PARTY_C_CONTEXT_ID/credentials" "$IH_AUTH" || true
echo "  credentials held: $(printf '%s' "$CALL_BODY" | jq -r 'length // 0' 2>/dev/null || echo '?')"

MGMT_AUTH="X-Api-Key: $CP_MANAGEMENT_KEY"

step "4. register the data plane with the control plane, and back"
call PUT "$CP_MANAGEMENT_URL/v4/dataplanes" "$MGMT_AUTH" "$(cat <<JSON
{
  "dataplaneId": "anonymous",
  "endpoint": "$DP_DATAFLOW_URL",
  "transferTypes": ["HttpData-PULL"],
  "labels": [],
  "authorization": { "type": "none" }
}
JSON
)"
call PUT "$DP_SIGNALING_URL/v1/controlplanes" "$MGMT_AUTH" "$(cat <<JSON
{
  "controlplaneId": "anonymous",
  "endpoint": "$CP_SIGNALING_URL",
  "authorization": { "type": "none" }
}
JSON
)"
echo "  ok"

step "5. asset, policy and contract definition for party A to consume"
# Create only what is missing. A second POST of an existing asset comes back as a 500 from
# a unique-constraint violation rather than a 409, so checking the status is not enough.
create_if_absent() {
  # `payload`, not `body`: call() and try_call() both assign to `body`, and shell
  # functions share globals, so the probe below would blank it out.
  kind="$1"; id="$2"; payload="$3"
  try_call GET "$CP_MANAGEMENT_URL/v4/$kind/$id" "$MGMT_AUTH"
  if [ "$CALL_STATUS" = "200" ]; then
    echo "  $kind/$id already exists, skipping"
    return 0
  fi
  call POST "$CP_MANAGEMENT_URL/v4/$kind" "$MGMT_AUTH" "$payload"
}

create_if_absent assets "$ASSET_ID" "$(cat <<JSON
{
  "@context": ["https://w3id.org/edc/connector/management/v2"],
  "@id": "$ASSET_ID",
  "@type": "Asset",
  "properties": { "description": "Party C's demo dataset, served by its data plane." }
}
JSON
)"
# One unconstrained permission: this direction tests protocol interop, not policy
# evaluation. It cannot be an empty rule set — DSP requires an Offer to carry at least one
# permission, prohibition or obligation, and a consumer validating against the schema
# rejects the catalog outright.
create_if_absent policydefinitions allow-all "$(cat <<'JSON'
{
  "@context": ["https://w3id.org/edc/connector/management/v2"],
  "@type": "PolicyDefinition",
  "@id": "allow-all",
  "policy": {
    "@type": "Set",
    "permission": [{ "action": "use" }],
    "prohibition": [],
    "obligation": []
  }
}
JSON
)"
create_if_absent contractdefinitions party-c-demo-def "$(cat <<JSON
{
  "@context": ["https://w3id.org/edc/connector/management/v2"],
  "@id": "party-c-demo-def",
  "@type": "ContractDefinition",
  "accessPolicyId": "allow-all",
  "contractPolicyId": "allow-all",
  "assetsSelector": {
    "@type": "Criterion",
    "operandLeft": "https://w3id.org/edc/v0.0.1/ns/id",
    "operator": "=",
    "operandRight": "$ASSET_ID"
  }
}
JSON
)"
echo "  ok"

step "party C seeded"
