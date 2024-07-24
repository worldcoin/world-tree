use std::fs;
use std::sync::Arc;

use ethers::providers::{Http, Provider};
use ethers_throttle::ThrottledJsonRpcClient;
use tree::config::ServiceConfig;
use tree::tree_manager::{BridgedTree, CanonicalTree, TreeManager};
use tree::WorldTree;

pub mod abi;
mod error;
pub mod serde_utils;
pub mod tree;

pub async fn init_world_tree(
    config: &ServiceConfig,
) -> eyre::Result<Arc<WorldTree<Provider<ThrottledJsonRpcClient<Http>>>>> {
    let canonical_provider_config = &config.canonical_tree.provider;

    let http_provider =
        Http::new(canonical_provider_config.rpc_endpoint.clone());
    let throttled_provider = ThrottledJsonRpcClient::new(
        http_provider,
        canonical_provider_config.throttle,
        None,
    );
    let canonical_middleware = Arc::new(Provider::new(throttled_provider));

    let canonical_tree_config = &config.canonical_tree;
    let canonical_tree_manager = TreeManager::<_, CanonicalTree>::new(
        canonical_tree_config.address,
        canonical_tree_config.window_size,
        canonical_tree_config.creation_block,
        canonical_middleware,
    )
    .await?;

    let mut bridged_tree_managers = vec![];

    for tree_config in config.bridged_trees.iter() {
        let bridged_provider_config = &tree_config.provider;
        let http_provider =
            Http::new(bridged_provider_config.rpc_endpoint.clone());

        let throttled_provider = ThrottledJsonRpcClient::new(
            http_provider,
            bridged_provider_config.throttle,
            None,
        );
        let bridged_middleware = Arc::new(Provider::new(throttled_provider));

        let tree_manager = TreeManager::<_, BridgedTree>::new(
            tree_config.address,
            tree_config.window_size,
            tree_config.creation_block,
            bridged_middleware,
        )
        .await?;

        bridged_tree_managers.push(tree_manager);
    }

    if config.cache.purge_cache {
        tracing::info!("Purging tree cache");
        if config.cache.cache_file.exists() {
            fs::remove_file(&config.cache.cache_file)?;
        }

        // There's something wrong with CascadingMerkleTree impl
        // in some cases it'll fail if the file doesn't exist
        fs::write(&config.cache.cache_file, [])?;
    }

    Ok(Arc::new(WorldTree::new(
        config.tree_depth,
        canonical_tree_manager,
        bridged_tree_managers,
        &config.cache.cache_file,
    )?))
}

#[cfg(test)]
mod tests {
    use tree::identity_tree::InclusionProof;
    use tree::Hash;

    use super::*;

    const PROOF: &str = indoc::indoc! {r#"{"root":"0x5341ac5400bb6222fe3591f0490f4e10a58f83a2fc1be793ab28b379bb00cbc","proof":[{"Left":"0x7f93e5f3fc398830ede0ed76b3e9860dd29a508c5a8cb5e3"},{"Right":"0x5bb361dda0cf5484cf3f64327d555db5c0307f4fe55df318f744b8dec0883d7"},{"Left":"0x4dac972dc548f1a83762a90641d0027ae11719d6a8e0f996cfbfc11736e4970"},{"Left":"0x28362e60ac85c736d69ece810e07b3b8e610dd68018f0a37d298cff455b5cf9b"},{"Right":"0x1bd3487dedc6e0faccb5172a3c3728a9b89fbed6c3a70dfb4f345127acda182d"},{"Left":"0xd3b49c1dbe26b454bb47a8f4d2708480493655b58c31dceebac53e9b03c001c"},{"Left":"0x2dee93c5a666459646ea7d22cca9e1bcfed71e6951b953611d11dda32ea09d78"},{"Left":"0x78295e5a22b84e982cf601eb639597b8b0515a88cb5ac7fa8a4aabe3c87349d"},{"Left":"0x2fa5e5f18f6027a6501bec864564472a616b2e274a41211a444cbe3a99f3cc61"},{"Right":"0x3c5b1dc55ce7fe648e320957a789dc40d8bc53303e16900e5d5bd99fb98c681"},{"Left":"0x1b7201da72494f1e28717ad1a52eb469f95892f957713533de6175e5da190af2"},{"Left":"0x1f8d8822725e36385200c0b201249819a6e6e1e4650808b5bebc6bface7d7636"},{"Left":"0x2c5d82f66c914bafb9701589ba8cfcfb6162b0a12acf88a8d0879a0471b5f85a"},{"Left":"0x14c54148a0940bb820957f5adf3fa1134ef5c4aaa113f4646458f270e0bfbfd0"},{"Left":"0x190d33b12f986f961e10c0ee44d8b9af11be25588cad89d416118e4bf4ebe80c"},{"Left":"0x22f98aa9ce704152ac17354914ad73ed1167ae6596af510aa5b3649325e06c92"},{"Left":"0x2a7c7c9b6ce5880b9f6f228d72bf6a575a526f29c66ecceef8b753d38bba7323"},{"Left":"0x2e8186e558698ec1c67af9c14d463ffc470043c9c2988b954d75dd643f36b992"},{"Left":"0xf57c5571e9a4eab49e2c8cf050dae948aef6ead647392273546249d1c1ff10f"},{"Left":"0x1830ee67b5fb554ad5f63d4388800e1cfe78e310697d46e43c9ce36134f72cca"},{"Left":"0x2134e76ac5d21aab186c2be1dd8f84ee880a1e46eaf712f9d371b6df22191f3e"},{"Left":"0x19df90ec844ebc4ffeebd866f33859b0c051d8c958ee3aa88f8f8df3db91a5b1"},{"Left":"0x18cca2a66b5c0787981e69aefd84852d74af0e93ef4912b4648c05f722efe52b"},{"Left":"0x2388909415230d1b4d1304d2d54f473a628338f2efad83fadf05644549d2538d"},{"Left":"0x27171fb4a97b6cc0e9e8f543b5294de866a2af2c9c8d0b1d96e673e4529ed540"},{"Left":"0x2ff6650540f629fd5711a0bc74fc0d28dcb230b9392583e5f8d59696dde6ae21"},{"Left":"0x120c58f143d491e95902f7f5277778a2e0ad5168f6add75669932630ce611518"},{"Left":"0x1f21feb70d3f21b07bf853d5e5db03071ec495a0a565a21da2d665d279483795"},{"Left":"0x24be905fa71335e14c638cc0f66a8623a826e768068a9e968bb1a1dde18a72d2"},{"Left":"0xf8666b62ed17491c50ceadead57d4cd597ef3821d65c328744c74e553dac26d"}]}"#};

    #[test]
    fn verify_inclusion_proof() {
        let leaf: Hash = "0x82162e98fe9a2be5cfe7ca07265641831aec6e235bc91122"
            .parse()
            .unwrap();
        let proof: InclusionProof = serde_json::from_str(PROOF).unwrap();

        let is_valid = proof.verify(leaf);

        println!("is_valid: {:?}", is_valid);
        panic!();
    }
}
