# world-tree

WorldId is one of the core pillars of the [Worldcoin Protocol](https://whitepaper.worldcoin.org/technical-implementation#worldcoin-protocol), providing a privacy preserving identity primitive which enables a robust mechanism for [proof of personhood](https://whitepaper.worldcoin.org/proof-of-personhood). The protocol works by maintaining an onchain representation of a [Semaphore](https://semaphore.pse.dev/docs/introduction) set, with each identity in the set representing a verified human. With WorldId, users can create Zero Knowledge proofs proving that they are a unique member of the set. In order to create a ZKP, users need to generate a merkle proof for their identity in the set. 


The `world-tree` is a service that syncs the state of the onchain merkle tree and delivers inclusion proofs for any given identity. 


## Installation
To install the `world-tree`, make sure you have [Rust installed](https://www.rust-lang.org/tools/install) and run the following commands.

```bash
git clone https://github.com/worldcoin/world-tree.git &&
cargo install --path .
```

## Usage
To run the `world-tree`, you can run the following command.

```bash
world-tree --config <path_to_config.toml>
```

To see an example configuration file, see `bin/world_tree.toml`. You can also specify the necessary configuration variables via environment variables.


<br>
<br>


## Docker usage & local testing
To run this service for local testing, you can execute the following command.

```
docker compose up -d --build
```

This command will build the docker image locally and start the service itself along with an anvil instance to serve as a test blockchain.

Anvil will fork Ethereum mainnet and use the production `WorldIdIdentityManager` contract, but with only a few identities inserted.

To test that inclusion proofs work you can execute the following curl command:

```
curl -X POST http://localhost:8080/inclusionProof -H "Content-Type: application/json" -d '{ "identityCommitment": "0x3017972D13A39795AD0D1C3A670D3D36A399B4435E61A510C2D57713D4F5C3DE" }'
```

