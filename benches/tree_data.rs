use criterion::async_executor::FuturesExecutor;
use criterion::{criterion_group, criterion_main, Criterion};
use rand::seq::IteratorRandom;
use rand::Rng;
use semaphore::lazy_merkle_tree::Canonical;
use world_tree::tree::tree_data::TreeData;
use world_tree::tree::{Hash, PoseidonTree};

pub const TREE_DEPTH: usize = 30;
pub const TREE_HISTORY_SIZE: usize = 24;
pub const NUMBER_OF_IDENTITIES: usize = 100;

fn generate_random_identity() -> Hash {
    let mut rng = rand::thread_rng();
    ruint::Uint::from(rng.gen::<usize>())
}

fn generate_random_identities(num_identities: usize) -> Vec<Hash> {
    let mut identities = vec![];

    for _ in 0..(num_identities) {
        identities.push(generate_random_identity());
    }

    identities
}
async fn setup_tree_data() -> TreeData {
    let tree = PoseidonTree::<Canonical>::new(TREE_DEPTH, Hash::ZERO);
    let tree_data = TreeData::new(tree, TREE_HISTORY_SIZE);

    tree_data
        .insert_many_at(0, &generate_random_identities(1 << 10))
        .await;

    tree_data
}

fn bench_insert_many_at(c: &mut Criterion) {
    let mut group = c.benchmark_group("TreeData Insertion");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tree_data = runtime.block_on(setup_tree_data());

    let identities = generate_random_identities(NUMBER_OF_IDENTITIES);

    group.bench_function("insert_many_at", |b| {
        b.to_async(FuturesExecutor)
            .iter(|| tree_data.insert_many_at(0, &identities))
    });
}

fn bench_delete_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("TreeData Deletion");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tree_data = runtime.block_on(setup_tree_data());

    let delete_indices: Vec<usize> = (0..NUMBER_OF_IDENTITIES).collect();

    group.bench_function("delete_many", |b| {
        b.to_async(FuturesExecutor)
            .iter(|| tree_data.delete_many(&delete_indices))
    });
}

fn bench_get_inclusion_proof_latest_root(c: &mut Criterion) {
    let mut group = c.benchmark_group("TreeData Inclusion Proof");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tree_data = runtime.block_on(setup_tree_data());

    let tree = runtime.block_on(tree_data.tree.read());

    let random_identity = tree
        .leaves()
        .choose(&mut rand::thread_rng())
        .expect("Could not get random identity");

    group.bench_function("get_inclusion_proof at latest root", |b| {
        b.to_async(FuturesExecutor)
            .iter(|| tree_data.get_inclusion_proof(random_identity, None))
    });
}

fn bench_get_inclusion_proof_historical_root(c: &mut Criterion) {
    let mut group =
        c.benchmark_group("get_inclusion_proof at oldest historical root");

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Insert the target identity
    let identity = generate_random_identity();
    let tree_data = runtime.block_on(setup_tree_data());
    runtime.block_on(tree_data.insert_many_at(0, &vec![identity]));

    // Update the tree history
    let tree_data = runtime.block_on(async {
        for _ in 0..TREE_HISTORY_SIZE {
            let identities = generate_random_identities(10);
            tree_data.insert_many_at(1, &identities).await;
        }

        tree_data
    });

    let tree_history = runtime.block_on(tree_data.tree_history.read());
    let oldest_root = tree_history.back().unwrap().root();

    group.bench_function("get_inclusion_proof at historical root", |b| {
        b.to_async(FuturesExecutor)
            .iter(|| tree_data.get_inclusion_proof(identity, Some(oldest_root)))
    });
}

criterion_group!(
    benches,
    bench_insert_many_at,
    bench_delete_many,
    bench_get_inclusion_proof_latest_root,
    bench_get_inclusion_proof_historical_root
);
criterion_main!(benches);
