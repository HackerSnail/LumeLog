//! Search throughput benchmark (spec P5). Manual run, not part of CI:
//! `cargo bench -p lumelog-core -- search_literal_1gb`
//!
//! Builds a ~1 GiB deterministic log, opens it once (index build is setup, not
//! measured), then times a whole-file literal scan. A rare pattern keeps the
//! per-match callback cost out of the measurement so the number reflects the
//! 8 MiB-windowed byte scan over a full gigabyte.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lumelog_core::doc::DocFile;
use lumelog_core::encoding::EncodingLabel;
use lumelog_core::search::{search_stream, Mode, SearchQuery};
use lumelog_core::tests_common::gen_log;
use std::io::Write;

fn search_literal_1gb(c: &mut Criterion) {
    let bytes = gen_log(1024); // ~1 GiB of ASCII HIL log lines
    let mut path = std::env::temp_dir();
    path.push(format!("lumelog_bench_{}_1gb.log", std::process::id()));
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&bytes).unwrap();
    }

    let mut doc = DocFile::open(&path, 2u64 << 30).unwrap();
    let q = SearchQuery {
        pattern_text: "val=999".to_string(), // ~1 hit per 1000 lines
        mode: Mode::Literal,
        encoding: EncodingLabel::Utf8,
        case_sensitive: true,
    };

    c.bench_function("search_literal_1gb", |b| {
        b.iter(|| {
            let stats = search_stream(&mut doc, &q, |_| {}, |_| true).unwrap();
            black_box(stats.matches)
        })
    });

    std::fs::remove_file(&path).ok();
}

criterion_group!(benches, search_literal_1gb);
criterion_main!(benches);
