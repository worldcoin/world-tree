use std::hint::black_box;

use semaphore::lazy_merkle_tree::Canonical;
use tree_availability::world_tree::tree_data::TreeData;
use tree_availability::world_tree::{Hash, PoseidonTree};

const NUM_IDENTITIES: usize = 10_000;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let tree =
        PoseidonTree::<Canonical>::new_with_dense_prefix(30, 20, &Hash::ZERO);

    let mut ref_tree =
        PoseidonTree::<Canonical>::new_with_dense_prefix(30, 20, &Hash::ZERO);

    let tree_data = TreeData::new(tree, 20_000);

    let identities: Vec<_> = (0..NUM_IDENTITIES).map(Hash::from).collect();

    let now = std::time::Instant::now();

    tree_data.insert_many_at(0, &identities[0..5000]).await;
    tree_data.gc().await;

    let time_to_first_gc = now.elapsed();

    for (idx, identity) in identities[0..5000].iter().enumerate() {
        ref_tree = ref_tree.update_with_mutation(idx, identity);
    }

    let root_1 = ref_tree.root();

    let now = std::time::Instant::now();

    tree_data.insert_many_at(5000, &identities[5000..]).await;
    tree_data.gc().await;

    let time_to_second_gc = now.elapsed();

    for (idx, identity) in identities[5000..].iter().enumerate() {
        ref_tree = ref_tree.update_with_mutation(idx + 5000, identity);
    }

    let root_2 = ref_tree.root();

    let mut inclusion_cases: Vec<(Hash, Hash)> =
        identities[0..5000].iter().map(|id| (*id, root_1)).collect();

    inclusion_cases
        .extend(identities[5000..].iter().map(|id| (*id, root_2.clone())));

    let now = std::time::Instant::now();

    for (identity, root) in &inclusion_cases {
        assert!(tree_data
            .get_inclusion_proof(*identity, Some(*root))
            .await
            .is_some());
    }

    let elapsed = now.elapsed();

    let total_time_to_update = time_to_first_gc + time_to_second_gc;
    println!(
        "Time to update {} identities: {:?}",
        NUM_IDENTITIES, total_time_to_update
    );

    println!(
        "Time to generate {} inclusion proofs: {:?}",
        inclusion_cases.len(),
        elapsed
    );

    Ok(())
}
