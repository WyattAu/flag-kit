use criterion::{criterion_group, criterion_main, Criterion};
use flag_kit::{Evaluator, Flag, FlagName, MemoryFlagStore};
use std::sync::Arc;

fn bench_flag_name_validation(c: &mut Criterion) {
    c.bench_function("flag_name_new_valid", |b| {
        b.iter(|| {
            let n = FlagName::new("my_feature_flag_123").unwrap();
            std::hint::black_box(n);
        });
    });
    c.bench_function("flag_name_new_invalid", |b| {
        b.iter(|| {
            let r = FlagName::new("Invalid-Flag");
            std::hint::black_box(r);
        });
    });
}

fn bench_bucket(c: &mut Criterion) {
    c.bench_function("bucket_hash", |b| {
        b.iter(|| {
            let v = flag_kit::bucket("my_flag", "user_12345");
            std::hint::black_box(v);
        });
    });
}

fn bench_memory_store(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("memory_store");

    group.bench_function("set", |b| {
        b.to_async(&rt).iter(|| async {
            let store = MemoryFlagStore::new();
            let flag = Flag::new(FlagName::new("bench_flag").unwrap(), true, 50).unwrap();
            store.set(flag).await.unwrap();
        });
    });

    group.bench_function("get_hit", |b| {
        b.to_async(&rt).iter(|| async {
            let store = MemoryFlagStore::new();
            let name = FlagName::new("bench_flag").unwrap();
            store
                .set(Flag::new(name.clone(), true, 50).unwrap())
                .await
                .unwrap();
            let got = store.get(&name).await;
            std::hint::black_box(got);
        });
    });

    group.bench_function("get_miss", |b| {
        b.to_async(&rt).iter(|| async {
            let store = MemoryFlagStore::new();
            let name = FlagName::new("missing").unwrap();
            let got = store.get(&name).await;
            std::hint::black_box(got);
        });
    });

    group.bench_function("list_100", |b| {
        b.to_async(&rt).iter(|| async {
            let store = MemoryFlagStore::new();
            for i in 0..100 {
                let name = FlagName::new(format!("flag_{i}")).unwrap();
                store
                    .set(Flag::new(name, true, 50).unwrap())
                    .await
                    .unwrap();
            }
            let list = store.list().await;
            std::hint::black_box(list);
        });
    });

    group.finish();
}

fn bench_evaluator(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("evaluator");

    group.bench_function("enabled", |b| {
        b.to_async(&rt).iter(|| async {
            let store = Arc::new(MemoryFlagStore::new());
            let name = FlagName::new("bench_flag").unwrap();
            store
                .set(Flag::new(name.clone(), true, 100).unwrap())
                .await
                .unwrap();
            let eval = Evaluator::new(store);
            let v = eval.enabled(&name).await;
            std::hint::black_box(v);
        });
    });

    group.bench_function("enabled_for_50pct", |b| {
        b.to_async(&rt).iter(|| async {
            let store = Arc::new(MemoryFlagStore::new());
            let name = FlagName::new("bench_flag").unwrap();
            store
                .set(Flag::new(name.clone(), true, 50).unwrap())
                .await
                .unwrap();
            let eval = Evaluator::new(store);
            let v = eval.enabled_for(&name, "user_123", None).await;
            std::hint::black_box(v);
        });
    });

    group.bench_function("enabled_for_all_100flags", |b| {
        b.to_async(&rt).iter(|| async {
            let store = Arc::new(MemoryFlagStore::new());
            for i in 0..100 {
                let name = FlagName::new(format!("flag_{i}")).unwrap();
                store
                    .set(Flag::new(name, true, 50).unwrap())
                    .await
                    .unwrap();
            }
            let eval = Evaluator::new(store);
            let v = eval.enabled_for_all("user_999", None).await;
            std::hint::black_box(v);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_flag_name_validation,
    bench_bucket,
    bench_memory_store,
    bench_evaluator
);
criterion_main!(benches);
