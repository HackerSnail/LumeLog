# Changelog

按 git log 时间倒序（最新在最上方）。每条写清：改了哪些文件、为什么、验证结果。

## 2026-10-24 — Task 4: DocFile 取块解码 + char_map

- 文件：新增 `src/doc/chunk.rs`（Chunk/CharMap/LogicalLine + build_chunk，含 GB18030 误切合并重试）、`src/doc/file.rs`（DocFile：open 探测编码+建索引 / set_encoding / chunk_at / 手写 VecDeque 64MB LRU / UTF-16 走 build_aligned 偶数对齐索引）、`doc/mod.rs` re-export；`tests/chunk_proptest.rs`（byte_to_line_cu 对齐 oracle）、`tests/docfile.rs`（临时文件集成）。
- 为什么：实现 spec §4.2 按行取块解码 + char_map（字节绝对偏移→(行,行内 cu16)，CodeMirror 坐标）；§5.2 行号与编码无关；§4.1 超 size_limit 拒开；Review Focus 1/5（split/对齐换算正确性）。
- 实现决策：
  - char_map v1 为“每逻辑行存 raw 跨度+解码文本，查询时逐字符走编码步进”（未做 ASCII 抽样压缩，已在 ledger 记 Ruling 缓至优化）；src_len：UTF-8 用 len_utf8、UTF-16 用 len_utf16*2、GBK/GB18030/BIG5/Shift-JIS 用逐字符再编码字节长（往返成立时精确）。
  - **修正 plan 假设**：plan Task4 Step1 设想“合并后 U+FFFD 消失”，但经实测 UTF-8/GBK/GB18030/BIG5/Shift-JIS 中 `0x0A` 从不是后续字节，合法流不可能被裸扫描误切；合并重试仅对畸形数据防御（测试改为“合法多行精确解码 + 畸形悬挂前导字节不 panic”）。
  - `finish` 契约（Task3 修订）在此消费：区域解码每块新建 IncrementalDecoder 从字节 0 语义→块内用 decode_all（BOM 仅文件首块）。
- 验证：`cargo test -p lumelog-core` → 26 lib + chunk_proptest(500) + docfile 5 + encoding_proptest 全 passed；`cargo clippy --all-targets -D warnings` 无告警。

### Task 4 修订（首轮 CodeReview 发现的缺陷）

- C-1 `chunk_at(_, 0)` 下流 panic（`line_start+count-1` 当 count=0）→ count==0 提前返回空 Chunk。
- I-1 `read_at` 短读被 `truncate` 静默吞掉→`build_chunk` 切片越界 panic（日志轮转/外部截断为主场景）→ `chunk_at` 校验 `bytes.len()==want`，不符返回 `CoreError::Io(UnexpectedEof)`；`to_rel` 加 `min` 二次防御。
- I-2 `decode_all` 对任意以 BOM 开头的切片都剥 BOM，与 `bom_skip` 仅 `ls==0` 不一致 → 非首块中部 `EF BB BF` 被误剥、文本丢一字符且 char_map 错位 → 新增 `encoding::decode_region`（从不剥 BOM），build_chunk 按 `ls==0` 选 decode_all/否则 decode_region。
- M-1 GB18030 合并重试循环对当前 9 种编码**不可达**（`0x0A` 从不是尾字节）→ 改为注释标注“对未来编码的防御”，测试新增断言 `logical_line_count==line_count`（证明未合并）。
- M-3 部分块（`start_line>0`）零覆盖→ chunk_proptest 新增 (D) 随机 start/count 子块与整块逐字节一致（正钉住 I-2）。
- 保留：M-2 UTF-16 整文件建索引的内存峰值（缓至流式 build_aligned 优化，ledger 记录）。
- 验证：`cargo test -p lumelog-core` → 36 全 passed（26 lib + 8 docfile + 2 proptest）；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 3 修订：CodeReview 发现的缺陷修复

- 文件：`src/encoding/mod.rs`、`src/doc/line_index.rs`、`tests/encoding_proptest.rs`。
- 为什么：首轮 CodeReview 实测暴露 3 Critical + 3 Important + 3 Minor，全部修复（不 park）。
  - Critical-1 `finish()` 丢弃 flush 文本 → `finish` 改为返回 `DecodeSegment`（带 text+invalid_count）。
  - Critical-2 flush 后复用 `encoding_rs::Decoder` 会 panic → `finish(mut self)` 消费所有权，杜绝复用。
  - Critical-3 BOM 跨小块、首块不足 bom_len 时漏剥 → 新增 `head` 缓冲累积到可判定再决定剥/BOM，`bom_done` 单次化。
  - Important：`invalid_count` 口径统一（decode_all 与逐段都数 U+FFFD，去掉 had_errors 门控）；detect 步骤2 容忍 64KB 窗口截断（`error_len().is_none()`）；proptest 空断言 → 语料补 UTF-16LE/BE+BOM、GBK/UTF-16 截尾、含字面 U+FFFD、随机字节，并消费新 finish 签名。
  - Minor：补 UTF-16BE 对齐扫描、builder >4GB 拒绝 TooLarge、UTF-16BE 往返+detect 测试。
- 验证：`cargo test -p lumelog-core` → 21 lib passed（含新确定性钉住用例 `bom_split_across_tiny_chunks_and_finish_flush`、`finish_returns_flushed_tail_text`，二轴覆盖 Critical-1/3）+ proptest(2000 cases) passed；`cargo clippy --all-targets -D warnings` 无告警。二次 CodeReview（区间 68168d9..8fa2178）裁定：6 项全修复、无新增 Critical/Important、Approved。

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
