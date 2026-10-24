use criterion::{criterion_group, criterion_main, Criterion};

fn placeholder(_c: &mut Criterion) {
    // Real benchmarks (search_literal_1gb, chunk_at_gb) land in Task 6/10.
}

criterion_group!(benches, placeholder);
criterion_main!(benches);
