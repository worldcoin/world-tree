#!/usr/bin/env sh

# Default values for host and port
HOST="0.0.0.0"
PORT="8545"
CONTRACT=""

# Function to display usage information
usage() {
  echo "Usage: $0 [-h host] [-p port] --contract contract_name" 1>&2
  exit 1
}

# Parse command-line options
while getopts ":h:p:-:" opt; do
  case "${opt}" in
    h)
      HOST=${OPTARG}
      ;;
    p)
      PORT=${OPTARG}
      ;;
    -)
      case "${OPTARG}" in
        contract)
          CONTRACT="${!OPTIND}"; OPTIND=$((OPTIND + 1))
          ;;
        *)
          usage
          ;;
      esac
      ;;
    *)
      usage
      ;;
  esac
done
shift $((OPTIND -1))

# Check if contract name is provided
if [ -z "${CONTRACT}" ]; then
  echo "Error: --contract argument is required" 1>&2
  usage
fi

# Start Anvil in the background
anvil --chain-id 31337 --block-time 2 --host ${HOST} --port ${PORT} &

ANVIL_PID=$!
PRIV_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Create contract
forge create --private-key $PRIV_KEY ${CONTRACT}

# Wait for anvil to finish
wait $ANVIL_PID
