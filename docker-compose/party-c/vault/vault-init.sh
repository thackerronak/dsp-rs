#!/bin/sh
# Seeds the secrets party C's runtimes read at boot. Dev-mode vault keeps nothing across
# restarts, so this runs on every `up` and is idempotent.
set -e

export VAULT_ADDR="${VAULT_ADDR:-http://party-c-vault:8200}"

echo "waiting for vault at $VAULT_ADDR"
until vault status >/dev/null 2>&1; do sleep 1; done

# Shared AES key for the connector's and IdentityHub's encryption at rest.
vault kv put secret/aes-key-alias content="${PARTY_C_AES_KEY}"

# Nothing else to seed: IdentityHub generates the STS client secret when the participant
# context is created and stores it under <participantContextId>-sts-client-secret, which
# is the alias the control plane is configured to read.

echo "vault seeded:"
vault kv list secret || true
