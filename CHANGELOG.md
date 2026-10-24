# Changelog

按 git log 时间倒序（最新在最上方）。每条写清：改了哪些文件、为什么、验证结果。

## 2026-10-24 — Task 1: Workspace 脚手架

- 文件：新增根 `Cargo.toml`（workspace, members=lumelog-core/lumelog-can, workspace.dependencies 锁 memchr/encoding_rs/chardetng/regex/thiserror）；`crates/lumelog-core` 的 Cargo.toml/src/lib.rs/src/tests_common.rs/benches/core_bench.rs；`crates/lumelog-can` 空占位；`.gitignore`。
- 为什么：M0 起点，重逻辑放纯 Rust crate，不依赖 Tauri，可 CLI 复用与基准测试（spec 第3节）。core_bench 占位使 cargo test 不因缺失 bench 目标报错。
- 验证：`cargo test -p lumelog-core` → crate_links 1 passed。
