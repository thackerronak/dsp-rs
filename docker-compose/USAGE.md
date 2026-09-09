# Using the Connectors to Negotiate Contracts and Transfer Data Assets

## Prerequisites

Follow [SETUP.md](SETUP.md) to:
1. Generate configuration: `./setup-configs.sh`
2. Start services: `docker-compose up -d`
3. Seed credentials into both connectors

## Catalog Sync

The connector for `party-a` provides a demo dataset under `party-a/data/datasets`. 
Upon syncing catalogs, `party-b` will discover and download the demo dataset to 
`party-b/data/datasets/federated/party-a`.

Sync authenticates like any other DSP call, so it only succeeds once both connectors
hold a credential. On a freshly started stack, the first sync runs before credentials 
are seeded and logs `Failed to retrieve an access token ... 401`; the next cycle picks 
it up. Until the dataset appears under `federated/party-a`, the negotiation below has 
nothing to negotiate for.

## Negotiating a contract

`party-b` is assumed to be the consumer, which starts a negotiation for the discovered dataset of `party-a`.

```sh
# start negotiation
NEGOTIATION_CONSUMER_PID=$(curl -s -H "Content-Type: application/json" http://localhost:23000/api-internal/negotiate \
  -d '{
    "name": "party-a", 
    "dataset_id": "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57",
    "offer_id": "urn:uuid:2828282:3dd1add8-4d2d-569e-d634-8394a8836a88"
  }' | jq -r '.consumer_pid')
echo "$NEGOTIATION_CONSUMER_PID"

# check status
AGREEMENT_ID=$(curl -s http://localhost:23000/api-internal/negotiate/$NEGOTIATION_CONSUMER_PID \
  | jq -r '.state | select(.type == "finalized") | .agreement["@id"]')
echo "$AGREEMENT_ID"
```

Note: you can check the full agreement by omiting the `jq` filter above:

```json
{
  "type": "consumer",
  "provider_pid": "urn:uuid:2f75b71d-276e-431f-97dd-ca46223f3819",
  "consumer_pid": "urn:uuid:fa150d2f-eb8d-434f-bd7a-91e3960ddc48",
  "state": {
    "type": "finalized",
    "agreement": {
      "@type": "Agreement",
      "@id": "urn:uuid:6f8fd102-11bc-408f-b489-52fe6353c5f2",
      "permission": [
        {
          "action": "use",
          "constraint": [
            {
              "leftOperand": "spatial",
              "operator": "isPartOf",
              "rightOperand": "_:EU"
            }
          ]
        }
      ],
      "target": "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57",
      "assigner": "did:web:party-a-connector%3A3000",
      "assignee": "did:web:party-b-connector%3A3000",
      "timestamp": "2026-08-31T11:06:23Z"
    }
  },
  "connector": {
    "connectorAddress": "http://party-a-connector:3000",
    "providerId": "party-a"
  }
}
```

## Start a transfer

```sh
# start transfer
TRANSFER_CONSUMER_PID=$(curl -s -H "Content-Type: application/json" http://localhost:23000/api-internal/transfer \
  -d '{
    "agreement_id": "'$AGREEMENT_ID'",
    "format": "HttpData-PULL"
  }' | jq -r '.consumer_pid')
echo "$TRANSFER_CONSUMER_PID"

# check status
curl -s http://localhost:23000/api-internal/transfer/$TRANSFER_CONSUMER_PID
```

Upon successful transfer start, the status should contain the endpoint and token for accessing the dataset:

```json
{
  "type": "consumer",
  "process": {
    "provider_pid": "urn:uuid:1536e189-df94-4e3b-8ff5-6544434dfad7",
    "consumer_pid": "urn:uuid:96fc6049-22e1-469b-aadc-31349f331f65",
    "state": {
      "type": "started",
      "sent": true
    }
  },
  "agreement": {
    "@type": "Agreement",
    "@id": "urn:uuid:6f8fd102-11bc-408f-b489-52fe6353c5f2",
    "permission": [
      {
        "action": "use",
        "constraint": [
          {
            "leftOperand": "spatial",
            "operator": "isPartOf",
            "rightOperand": "_:EU"
          }
        ]
      }
    ],
    "target": "urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57",
    "assigner": "did:web:party-a-connector%3A3000",
    "assignee": "did:web:party-b-connector%3A3000",
    "timestamp": "2026-08-31T11:06:23Z"
  },
  "format": "HttpData-PULL",
  "connector": {
    "connectorAddress": "http://party-a-connector:3000",
    "providerId": "party-a"
  },
  "data_address": {
    "@type": "DataAddress",
    "endpointType": "https://w3id.org/idsa/v4.1/HTTP",
    "endpoint": "http://party-a-connector:3000/pull",
    "endpointProperties": [
      {
        "@type": "EndpointProperty",
        "name": "authorization",
        "value": "eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ1cm46dXVpZDoxNTM2ZTE4OS1kZjk0LTRlM2ItOGZmNS02NTQ0NDM0ZGZhZDciLCJpc3MiOiJkaWQ6d2ViOnBhcnR5LWEtY29ubmVjdG9yJTNBMzAwMCIsImp0aSI6IjQyMDcwOWE4LTYxMzMtNDhiZi1iM2RmLTMyZGI1NDc3M2RiNSIsImlhdCI6MTc4ODE3NDkzNCwiZXhwIjoxNzg4MjAzNzM0fQ.DV9ceG85wYc1KJByPOGJVI_5xhybS2bevrj6lCBoCV0Pe3CgHGY-XU17PAWwf_vCrlmYJrZ-x8zMvxB0B0S0ZA"
      },
      {
        "@type": "EndpointProperty",
        "name": "authType",
        "value": "bearer"
      }
    ]
  }
}
```

## Access dataset/service

NOTE: we have to map the returned endpoint address of `party-a` to localhost:
`http://party-a-connector:3000/pull --> http://localhost:13000/pull`

```sh
curl -s \
  -H "Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ1cm46dXVpZDoxNTM2ZTE4OS1kZjk0LTRlM2ItOGZmNS02NTQ0NDM0ZGZhZDciLCJpc3MiOiJkaWQ6d2ViOnBhcnR5LWEtY29ubmVjdG9yJTNBMzAwMCIsImp0aSI6IjQyMDcwOWE4LTYxMzMtNDhiZi1iM2RmLTMyZGI1NDc3M2RiNSIsImlhdCI6MTc4ODE3NDkzNCwiZXhwIjoxNzg4MjAzNzM0fQ.DV9ceG85wYc1KJByPOGJVI_5xhybS2bevrj6lCBoCV0Pe3CgHGY-XU17PAWwf_vCrlmYJrZ-x8zMvxB0B0S0ZA" \
  -H "Content-Type: application/json" \
  http://localhost:13000/pull/test-path?test-query=true \
  -d '{"sample-payload": 1}'
```

Upon success, the echo service behind the demo dataset should echo the request:

```json
{
  "method": "POST",
  "path": "/api/test-path",
  "query": {
    "test-query": "true"
  },
  "headers": {
    "host": "localhost:13000",
    "content-length": "21",
    "user-agent": "curl/8.7.1",
    "accept": "*/*",
    "content-type": "application/json"
  },
  "body": "eyJzYW1wbGUtcGF5bG9hZCI6IDF9"
}
```

## Interoperating with party C (Java EDC)

Everything above is party B talking to party A: the same implementation on both ends. The
flows below are the same protocols against the Eclipse EDC stack, in both directions. Bring
party C up and seed it first — see [SETUP.md](SETUP.md#party-c--a-java-edc-connector).

### Party C consumes from party A

Party C drives this through its management API, which is key-protected
(`X-Api-Key: password`). Note `counterPartyAddress` is party A's **complete** DSP endpoint,
version path included.

```sh
CP=http://localhost:34081/api/mgmt
A_DSP=http://party-a-connector:3000/api/2025/1
A_DID='did:web:party-a-connector%3A3000'

# 1. catalog
curl -s -X POST "$CP/v4/catalog/request" -H 'X-Api-Key: password' \
  -H 'content-type: application/json' -d "{
    \"@context\": [\"https://w3id.org/edc/connector/management/v2\"],
    \"@type\": \"CatalogRequest\",
    \"counterPartyAddress\": \"$A_DSP\",
    \"counterPartyId\": \"$A_DID\",
    \"protocol\": \"dataspace-protocol-http:2025-1\"
  }" | jq '.dataset.hasPolicy[0]."@id"'

# 2. negotiate — echo the offer from the catalog, with assigner and target filled in
curl -s -X POST "$CP/v4/contractnegotiations" -H 'X-Api-Key: password' \
  -H 'content-type: application/json' -d "{
    \"@context\": [\"https://w3id.org/edc/connector/management/v2\"],
    \"@type\": \"ContractRequest\",
    \"counterPartyAddress\": \"$A_DSP\",
    \"counterPartyId\": \"$A_DID\",
    \"protocol\": \"dataspace-protocol-http:2025-1\",
    \"policy\": {
      \"@type\": \"Offer\",
      \"@id\": \"urn:uuid:2828282:3dd1add8-4d2d-569e-d634-8394a8836a88\",
      \"assigner\": \"$A_DID\",
      \"target\": \"urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57\",
      \"permission\": [{\"action\": \"use\", \"constraint\": [
        {\"leftOperand\": \"spatial\", \"operator\": \"isPartOf\", \"rightOperand\": \"_:EU\"}
      ]}],
      \"prohibition\": [], \"obligation\": []
    }
  }" | jq -r '."@id"'

# 3. wait for FINALIZED and take the agreement id
curl -s -H 'X-Api-Key: password' "$CP/v4/contractnegotiations/<negotiation-id>" \
  | jq '{state, contractAgreementId}'

# 4. transfer
curl -s -X POST "$CP/v4/transferprocesses" -H 'X-Api-Key: password' \
  -H 'content-type: application/json' -d "{
    \"@context\": [\"https://w3id.org/edc/connector/management/v2\"],
    \"@type\": \"TransferRequest\",
    \"assetId\": \"urn:uuid:3afeadd8-ed2d-569e-d634-8394a8836d57\",
    \"counterPartyAddress\": \"$A_DSP\",
    \"connectorId\": \"$A_DID\",
    \"contractId\": \"<agreement-id>\",
    \"dataDestination\": {\"@type\": \"DataAddress\", \"type\": \"HttpProxy\"},
    \"protocol\": \"dataspace-protocol-http:2025-1\",
    \"transferType\": \"HttpData-PULL\"
  }" | jq -r '."@id"'
```

Once the transfer reaches `STARTED`, party C's data plane holds the data address party A
sent:

```sh
curl -s http://localhost:34103/api/proxy/flows/<transfer-id> | jq
```

The `authorization` property carries the bearer token. **`endpoint` comes back null**: party
A sends it as `dspace:endpoint`, which is what DSP defines, but this EDC release reads the
endpoint from its own namespace and drops it. So pull from party A directly, using the
token party C received:

```sh
TOKEN=$(curl -s http://localhost:34103/api/proxy/flows/<transfer-id> \
  | jq -r '.endpointProperties[] | select(.name=="authorization") | .value')

curl -s -X POST 'http://localhost:13000/pull/test-path?test-query=true' \
  -H "Authorization: Bearer $TOKEN" | jq
```

### Party A consumes from party C

Party A discovers party C's DSP endpoint from the `DataService` entry in party C's DID
document and syncs its catalog on the federation interval, so party C's asset shows up
under `party-a/connector/data/datasets/federated/party-c/`. Restart party A if you do not
want to wait out the interval.

```sh
OFFER=$(jq -r '.dataset.hasPolicy[0]."@id"' \
  party-a/connector/data/datasets/federated/party-c/party-c-asset-1.json)

# negotiate
curl -s -X POST http://localhost:13000/api-internal/negotiate \
  -H 'content-type: application/json' \
  -d "{\"name\":\"party-c\",\"dataset_id\":\"party-c-asset-1\",\"offer_id\":\"$OFFER\"}"

# wait for "finalized", then take the agreement id
curl -s http://localhost:13000/api-internal/negotiate/<consumer-pid> | jq '.state'

# transfer
curl -s -X POST http://localhost:13000/api-internal/transfer \
  -H 'content-type: application/json' \
  -d '{"agreement_id":"<agreement-id>","format":"HttpData-PULL"}'
```

Party C sends a complete data address, so the pull needs nothing but a host-port rewrite:

```sh
D=$(curl -s http://localhost:13000/api-internal/transfer/<consumer-pid>)
TOKEN=$(echo "$D" | jq -r '.data_address.endpointProperties[] | select(.name=="access_token") | .value')
EP=$(echo "$D" | jq -r '.data_address.endpoint' \
  | sed 's|http://party-c-dataplane:11002|http://localhost:34102|')

curl -s "$EP" -H "Authorization: Bearer $TOKEN" | jq
```
