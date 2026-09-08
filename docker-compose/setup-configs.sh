#!/bin/bash
# Setup script to generate connector configs from templates using .env

set -e

echo "========================================
Setting up connector configurations
========================================"
echo ""

# Check if .env exists, if not create from .env.sample
if [ ! -f .env ]; then
  if [ -f .env.sample ]; then
    echo "Creating .env from .env.sample..."
    cp .env.sample .env
    echo "✓ .env created (defaults to localhost)"
    echo ""
    echo "⚠️  Edit .env if you need to:"
    echo "   - Use NGrok: set ENFORCE_HTTPS=true and add auth tokens/domains"
    echo "   - Use custom keys: update PARTY_*_PRIVATE_KEY_PEM"
    echo ""
  else
    echo "Error: .env and .env.sample not found"
    exit 1
  fi
fi

echo "Loading environment variables from .env..."

# Parse .env file safely
while IFS='=' read -r key value; do
  # Skip comments and empty lines
  [[ "$key" =~ ^#.*$ ]] && continue
  [[ -z "$key" ]] && continue

  # Remove quotes if present
  value=${value%\"}
  value=${value#\"}

  # Store in array for later use
  export "$key"="$value"
done < .env

# Check ENFORCE_HTTPS flag (default: false)
ENFORCE_HTTPS=${ENFORCE_HTTPS:-false}

echo "Configuration Mode:"
echo "  ENFORCE_HTTPS=$ENFORCE_HTTPS"

if [ "$ENFORCE_HTTPS" = "true" ]; then
  echo "  → Using NGrok HTTPS tunnels"

  # Verify required NGrok variables
  required_vars=(
    "PARTY_A_NGROK_DOMAIN"
    "PARTY_B_NGROK_DOMAIN"
    "PARTY_A_NGROK_AUTHTOKEN"
    "PARTY_B_NGROK_AUTHTOKEN"
  )

  for var in "${required_vars[@]}"; do
    if [ -z "${!var}" ]; then
      echo "Error: Required NGrok variable $var is not set"
      echo "       Set ENFORCE_HTTPS=false for localhost, or provide NGrok tokens and domains"
      exit 1
    fi
  done

  # Set addresses for HTTPS NGrok
  export PARTY_A_EXTERNAL_ADDRESS="https://${PARTY_A_NGROK_DOMAIN}"
  export PARTY_A_DID_HOST="${PARTY_A_NGROK_DOMAIN}"
  export PARTY_B_EXTERNAL_ADDRESS="https://${PARTY_B_NGROK_DOMAIN}"
  export PARTY_B_DID_HOST="${PARTY_B_NGROK_DOMAIN}"

  echo "  Party A: $PARTY_A_EXTERNAL_ADDRESS"
  echo "  Party B: $PARTY_B_EXTERNAL_ADDRESS"
else
  echo "  → Using Localhost HTTP (default for development)"

  # Set addresses for localhost
  export PARTY_A_EXTERNAL_ADDRESS="http://party-a-connector:3000"
  export PARTY_A_DID_HOST="party-a-connector:3000"
  export PARTY_B_EXTERNAL_ADDRESS="http://party-b-connector:3000"
  export PARTY_B_DID_HOST="party-b-connector:3000"

  echo "  Party A: $PARTY_A_EXTERNAL_ADDRESS"
  echo "  Party B: $PARTY_B_EXTERNAL_ADDRESS"
fi

# Verify common required variables
common_required_vars=(
  "PARTY_A_PRIVATE_KEY_PEM"
  "PARTY_B_PRIVATE_KEY_PEM"
)

for var in "${common_required_vars[@]}"; do
  if [ -z "${!var}" ]; then
    echo "Error: Required variable $var is not set"
    exit 1
  fi
done

echo ""
echo "Generating Party A connector config..."
sed -e "s|\${PARTY_A_EXTERNAL_ADDRESS}|${PARTY_A_EXTERNAL_ADDRESS}|g" \
    -e "s|\${PARTY_A_DID_HOST}|${PARTY_A_DID_HOST}|g" \
    -e "s|\${PARTY_B_DID_HOST}|${PARTY_B_DID_HOST}|g" \
    -e "s|\${PARTY_A_PRIVATE_KEY_PEM}|${PARTY_A_PRIVATE_KEY_PEM}|g" \
    -e "s|\${PARTY_A_WALLET_PRIVATE_KEY_PEM}|${PARTY_A_WALLET_PRIVATE_KEY_PEM}|g" \
    -e "s|\${WALLET_CREDENTIAL_STORE_PATH}|${WALLET_CREDENTIAL_STORE_PATH}|g" \
    -e "s|\${WALLET_STS_CLIENT_ID}|${WALLET_STS_CLIENT_ID}|g" \
    -e "s|\${WALLET_STS_CLIENT_SECRET}|${WALLET_STS_CLIENT_SECRET}|g" \
    party-a/connector/config.json.template > party-a/connector/config.json

echo "Generating Party B connector config..."
sed -e "s|\${PARTY_B_EXTERNAL_ADDRESS}|${PARTY_B_EXTERNAL_ADDRESS}|g" \
    -e "s|\${PARTY_A_DID_HOST}|${PARTY_A_DID_HOST}|g" \
    -e "s|\${PARTY_B_DID_HOST}|${PARTY_B_DID_HOST}|g" \
    -e "s|\${PARTY_B_PRIVATE_KEY_PEM}|${PARTY_B_PRIVATE_KEY_PEM}|g" \
    -e "s|\${PARTY_B_WALLET_PRIVATE_KEY_PEM}|${PARTY_B_WALLET_PRIVATE_KEY_PEM}|g" \
    -e "s|\${WALLET_CREDENTIAL_STORE_PATH}|${WALLET_CREDENTIAL_STORE_PATH}|g" \
    -e "s|\${WALLET_STS_CLIENT_ID}|${WALLET_STS_CLIENT_ID}|g" \
    -e "s|\${WALLET_STS_CLIENT_SECRET}|${WALLET_STS_CLIENT_SECRET}|g" \
    party-b/connector/config.json.template > party-b/connector/config.json

echo "✓ Configurations generated successfully"
echo ""
echo "Next step:"
if [ "$ENFORCE_HTTPS" = "true" ]; then
  echo "  docker-compose --profile ngrok up -d"
else
  echo "  docker-compose up -d"
fi
