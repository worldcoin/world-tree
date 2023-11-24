use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
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

fn setup_tree_data() -> TreeData {
    let tree = PoseidonTree::<Canonical>::new(TREE_DEPTH, Hash::ZERO);
    let mut tree_data = TreeData::new(tree, TREE_HISTORY_SIZE);

    tree_data.insert_many_at(0, &generate_random_identities(1 << 10));
    tree_data
}

fn bench_insert_many_at(c: &mut Criterion) {
    let mut group = c
        .benchmark_group(format!("Insert {} identities", NUMBER_OF_IDENTITIES));

    let tree_data = setup_tree_data();
    group.bench_function("insert_many_at", |b| {
        b.iter_batched(
            || {
                (
                    tree_data.clone(),
                    generate_random_identities(NUMBER_OF_IDENTITIES),
                )
            },
            |(mut tree_data, identities)| {
                tree_data.insert_many_at(0, &identities);
            },
            BatchSize::SmallInput,
        );
    });
}
fn bench_delete_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_many");

    let tree_data = setup_tree_data();
    let delete_indices: Vec<usize> = (0..NUMBER_OF_IDENTITIES).collect();

    group.bench_function("delete_many", |b| {
        b.iter_batched(
            || (tree_data.clone(), delete_indices.clone()),
            |(mut tree_data, delete_indices)| {
                tree_data.delete_many(&delete_indices);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_get_inclusion_proof_latest_root(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_inclusion_proof at latest root");

    let tree_data = setup_tree_data();

    let random_identity = tree_data
        .tree
        .leaves()
        .choose(&mut rand::thread_rng())
        .expect("Could not get random identity");

    group.bench_function("get_inclusion_proof at latest root", |b| {
        b.iter_batched(
            || (tree_data.clone(), random_identity),
            |(tree_data, random_identity)| {
                tree_data.get_inclusion_proof(random_identity, None);
            },
            BatchSize::SmallInput,
        );
    });
}
fn bench_get_inclusion_proof_historical_root(c: &mut Criterion) {
    let mut group =
        c.benchmark_group("get_inclusion_proof at oldest historical root");

    // Setup a common tree data state for all iterations
    let mut tree_data = setup_tree_data();

    // Insert the target identity
    let identity = generate_random_identity();
    tree_data.insert_many_at(0, &vec![identity]);

    // Update the tree history
    for _ in 0..TREE_HISTORY_SIZE {
        let identities = generate_random_identities(10);
        tree_data.insert_many_at(1, &identities);
    }

    // Get the oldest root for the benchmark
    let oldest_root = tree_data.tree_history.back().unwrap().tree.root();

    group.bench_function("get_inclusion_proof at historical root", |b| {
        b.iter_batched(
            || (tree_data.clone(), identity, oldest_root),
            |(tree_data, identity, oldest_root)| {
                tree_data.get_inclusion_proof(identity, Some(oldest_root));
            },
            BatchSize::SmallInput,
        );
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
