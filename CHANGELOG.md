# Changelog

按 git log 时间倒序（最新在最上方）。每条写清：改了哪些文件、为什么、验证结果。

## 2026-10-24 — Task 3 修订：CodeReview 发现的缺陷修复

- 文件：`src/encoding/mod.rs`、`src/doc/line_index.rs`、`tests/encoding_proptest.rs`。
- 为什么：首轮 CodeReview 实测暴露 3 Critical + 3 Important + 3 Minor，全部修复（不 park）。
  - Critical-1 `finish()` 丢弃 flush 文本 → `finish` 改为返回 `DecodeSegment`（带 text+invalid_count）。
  - Critical-2 flush 后复用 `encoding_rs::Decoder` 会 panic → `finish(mut self)` 消费所有权，杜绝复用。
  - Critical-3 BOM 跨小块、首块不足 bom_len 时漏剥 → 新增 `head` 缓冲累积到可判定再决定剥/BOM，`bom_done` 单次化。
  - Important：`invalid_count` 口径统一（decode_all 与逐段都数 U+FFFD，去掉 had_errors 门控）；detect 步骤2 容忍 64KB 窗口截断（`error_len().is_none()`）；proptest 空断言 → 语料补 UTF-16LE/BE+BOM、GBK/UTF-16 截尾、含字面 U+FFFD、随机字节，并消费新 finish 签名。
  - Minor：补 UTF-16BE 对齐扫描、builder >4GB 拒绝 TooLarge、UTF-16BE 往返+detect 测试。
- 验证：`cargo test -p lumelog-core` → 19 lib passed + proptest(2000 cases) passed；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 3: 编码模块

- 文件：新增 `src/encoding/mod.rs`（EncodingLabel 9 种显示名 + as_str/from_label、detect、decode_all、IncrementalDecoder、encode_all）、`tests/encoding_proptest.rs`；lib.rs 挂 `pub mod encoding`。
- 为什么：实现 spec §5 检测顺序（BOM→严格 UTF-8→chardetng→回退 GB18030）与 §5.4 U+FFFD 错误策略；GB2312→GBK 超集（A3）；增量解码器为 Task4 分块取行服务，split-invariance 是正确性根基。
  - chardetng 0.1.17 无 float 置信度，用 `guess_assess(Some(b"cn"), true)` 的 plausible 标志实现 §5.2 的“置信阈值”（allow_utf8=true 让被截断的 UTF-8 头仍可落到 UTF-8）。
- 验证：`cargo test -p lumelog-core` → 16 lib passed + proptest split_anywhere passed；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 2: LineIndex 行索引

- 文件：新增 `src/error.rs`（CoreError：Io/TooLarge/LineOutOfRange/Encoding/Conflict/Unmappable/Locked）、`src/doc/line_index.rs`、`src/doc/mod.rs`；lib.rs 挂 `pub mod doc; pub mod error;`。
- 为什么：行边界按字节（仅 0x0A 分割），与编码无关 → 切编码保留行号（spec §5.2）；u32 偏移钉死 4GB 上限（§4.1/A9）；空文件=1行、CRLF 归上一行、UTF-16 偶数对齐扫描（Review Focus 3/5）。
- 验证：`cargo test -p lumelog-core --lib` → 8 passed（含 builder_chunks_equal_whole、utf16le_scan_even_aligned_only、gb18030 误切可索引）；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 1: Workspace 脚手架

- 文件：新增根 `Cargo.toml`（workspace, members=lumelog-core/lumelog-can, workspace.dependencies 锁 memchr/encoding_rs/chardetng/regex/thiserror）；`crates/lumelog-core` 的 Cargo.toml/src/lib.rs/src/tests_common.rs/benches/core_bench.rs；`crates/lumelog-can` 空占位；`.gitignore`。
- 为什么：M0 起点，重逻辑放纯 Rust crate，不依赖 Tauri，可 CLI 复用与基准测试（spec 第3节）。core_bench 占位使 cargo test 不因缺失 bench 目标报错。
- 验证：`cargo test -p lumelog-core` → crate_links 1 passed。
