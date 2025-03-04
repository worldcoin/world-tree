#!/usr/bin/env sh

# Default values for host, port, and chain id
HOST="0.0.0.0"
PORT="8545"
CHAIN_ID="31337"
CONTRACT=""

# Function to display usage information
usage() {
  echo "Usage: $0 [-h host] [-p port] [-c chain_id] --contract contract_name" 1>&2
  exit 1
}

# Parse command-line options
while [ "$#" -gt 0 ]; do
  case "$1" in
  -h)
    HOST="$2"
    shift 2
    ;;
  -p)
    PORT="$2"
    shift 2
    ;;
  -c)
    CHAIN_ID="$2"
    shift 2
    ;;
  --contract)
    CONTRACT="$2"
    shift 2
    ;;
  *)
    usage
    ;;
  esac
done

# Check if contract name is provided
if [ -z "${CONTRACT}" ]; then
  echo "Error: --contract argument is required" 1>&2
  usage
fi

# Start Anvil in the background
anvil --chain-id ${CHAIN_ID} --block-time 2 --host ${HOST} --port ${PORT} &

ANVIL_PID=$!
PRIV_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Wait until Anvil starts in the background
while ! bash -c "echo > /dev/tcp/${HOST}/${PORT}" 2>/dev/null; do
  sleep 1
done

# Create contract
forge create --broadcast --private-key $PRIV_KEY ${CONTRACT}

# Wait for anvil to finish
wait $ANVIL_PID
