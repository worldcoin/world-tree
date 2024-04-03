use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use rand::Rng;
use world_tree::tree::identity_tree::IdentityTree;
use world_tree::tree::Hash;

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

fn bench_insert_identities(c: &mut Criterion) {
    let mut group = c
        .benchmark_group(format!("Insert {} identities", NUMBER_OF_IDENTITIES));

    group.bench_function("insert_many_at", |b| {
        b.iter_batched(
            || {
                (
                    IdentityTree::new(TREE_DEPTH),
                    generate_random_identities(NUMBER_OF_IDENTITIES),
                )
            },
            |(mut tree, leaves)| {
                for (idx, leaf) in leaves.iter().enumerate() {
                    tree.insert(idx as u32, *leaf);
                }
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_insert_identities,);
criterion_main!(benches);
