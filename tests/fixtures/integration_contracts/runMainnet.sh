#!/usr/bin/env sh

# Start Anvil in the background
anvil --host 0.0.0.0 &

ANVIL_PID=$!
PRIV_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Create WorldIDIdentityManager
# Expected address: 0x5FbDB2315678afecb367f032d93F642f64180aa3
forge create --private-key $PRIV_KEY WorldIDIdentityManager

# Wait for anvil to finish
wait $ANVIL_PID
