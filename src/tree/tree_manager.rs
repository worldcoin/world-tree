use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::DerefMut;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use common::test_utilities::abi::{
    DeleteIdentitiesCall, RegisterIdentitiesCall,
};
use ethers::abi::AbiDecode;
use ethers::contract::{ContractError, EthCall, EthEvent};
use ethers::providers::{Middleware, MiddlewareError, StreamExt};
use ethers::types::{
    Filter, Log, Selector, Transaction, ValueOrArray, H160, U256,
};
use futures::stream::FuturesUnordered;
use ruint::Uint;
use semaphore::dynamic_merkle_tree::DynamicMerkleTree;
use semaphore::poseidon_tree::PoseidonHash;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;
use tracing::instrument;

use super::block_scanner::BlockScanner;
use super::error::TreeAvailabilityError;
use super::identity_tree::{IdentityUpdates, Root};
use super::tree_data::TreeData;
use super::Hash;
use crate::abi::{
    DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall,
    IBridgedWorldID, TreeChangedFilter,
};

pub struct TreeManager<M: Middleware> {
    pub canonical_tree_manager: CanonicalTreeManager<M>,
    pub bridged_tree_manager: Vec<BridgedTreeManager<M>>,
}

impl<M> TreeManager<M>
where
    M: Middleware,
{
    //TODO: return join handles and receivers
    pub async fn spawn(&self) -> eyre::Result<()> {
        todo!();
    }
}

impl<M> CanonicalTreeManager<M>
where
    M: Middleware,
{
    async fn spawn() {
        //TODO: poll for updates for each of these, sending data down the channels
    }
}

//TODO: ^^ write some spawn function that spawns a new block scanner, then sends the id updates through the tx
pub struct CanonicalTreeManager<M: Middleware> {
    pub identity_update_tx: Sender<(Root, IdentityUpdates)>,
    pub address: H160,
    pub block_scanner: BlockScanner<M>,
    pub chain_id: u64,
}

impl<M> CanonicalTreeManager<M>
where
    M: Middleware,
{
    async fn poll_for_updates() {}
}

pub struct BridgedTreeManager<M: Middleware> {
    pub root_tx: Sender<Root>,
    pub address: H160,
    pub block_scanner: BlockScanner<M>,
    pub chain_id: u64,
}

impl<M> BridgedTreeManager<M>
where
    M: Middleware,
{
    async fn poll_for_updates() {}
}

//TODO: ^^ write some spawn function that spawns a new block scanner, then sends the root for that chain through the tx with the chainid. TODO: get the chainid for the middleware upon spawning

//- first we get the state of all bridged tree roots to get the latest root
//- then we sync the state of the canonical tree to the latest root, caching the updates from the lcd root along the way

pub async fn extract_identity_updates<M: Middleware + 'static>(
    logs: &[Log],
    middleware: Arc<M>,
) -> eyre::Result<BTreeMap<Root, IdentityUpdates>> {
    let mut tree_updates = BTreeMap::new();

    //TODO: you can do this concurrently
    for log in logs {
        let block_number =
            log.block_number.expect("TODO: handle this case").as_u64();

        let root = Root {
            root: Hash::from_le_bytes(log.topics[3].0),
            block_number,
        };

        let mut identity_updates = HashMap::new();

        let tx_hash = log.transaction_hash.expect("TODO: handle this case");

        tracing::info!(?tx_hash, "Getting transaction");

        let transaction = middleware
            .get_transaction(tx_hash)
            .await?
            .expect("TODO: Handle this case");

        let calldata = &transaction.input;

        let function_selector = Selector::try_from(&calldata[0..4])
            .expect("Transaction data does not contain a function selector");

        if function_selector == RegisterIdentitiesCall::selector() {
            tracing::info!("Decoding registerIdentities calldata");

            let register_identities_call =
                RegisterIdentitiesCall::decode(calldata.as_ref())?;

            let start_index = register_identities_call.start_index;
            let identities = register_identities_call.identity_commitments;

            for (i, identity) in identities.into_iter().take_while(|x| *x != U256::zero()).enumerate(){
                
                identity_updates.insert(start_index +  i as u32, Hash::from_limbs(identity.0));
            }


        } else if function_selector == DeleteIdentitiesCall::selector() {
            tracing::info!("Decoding deleteIdentities calldata");

            let delete_identities_call =
                DeleteIdentitiesCall::decode(calldata.as_ref())?;

            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );


            //TODO: note that we use 2**30 for padding index
                for i in indices.into_iter().take_while(|x| *x < 2_u32.pow(30)){
                    identity_updates.insert(  i , Hash::ZERO);
                }

    

        } else if function_selector == DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::selector() {

            tracing::info!("Decoding deleteIdentities calldata");

            // @dev This is a type that is generated by abigen!() since there is a function defined with a conflicting function name but different params
            let delete_identities_call =
            DeleteIdentitiesWithDeletionProofAndBatchSizeAndPackedDeletionIndicesAndPreRootCall::decode(calldata.as_ref())?;
            let indices= unpack_indices(
                delete_identities_call.packed_deletion_indices.as_ref(),
            );


        //TODO: note that we use 2**30 for padding index
        for i in indices.into_iter().take_while(|x| *x < 2_u32.pow(30)){
            identity_updates.insert(  i , Hash::ZERO);
        }
    }

        tree_updates.insert(root, identity_updates);
    }

    Ok(tree_updates)
}

/// Unpacks a contiguous byte array into a vector of 32-bit indices.
///
/// # Arguments
///
/// * `packed` - The packed byte array containing positions of identity commitments in the `WorldTree`.
pub fn unpack_indices(packed: &[u8]) -> Vec<u32> {
    let mut indices = Vec::with_capacity(packed.len() / 4);

    for packed_index in packed.chunks_exact(4) {
        let index = u32::from_be_bytes(
            packed_index.try_into().expect("Invalid index length"),
        );

        indices.push(index);
    }

    indices
}

/// Packs an array of 32-bit indices into a contiguous byte vector.
///
/// # Arguments
///
/// * `indices` - The array of indices representing positions of identity commitments in the `WorldTree`.
pub fn pack_indices(indices: &[u32]) -> Vec<u8> {
    let mut packed = Vec::with_capacity(indices.len() * 4);

    for index in indices {
        packed.extend_from_slice(&index.to_be_bytes());
    }

    packed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_indices() {
        let indices = vec![1, 2, 3, 4, 5, 6, 7, 8];

        let packed = pack_indices(&indices);

        assert_eq!(packed.len(), 32);

        let unpacked = unpack_indices(&packed);

        assert_eq!(unpacked, indices);
    }
}
