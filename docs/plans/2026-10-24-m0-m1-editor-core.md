# LumeLog Plan 1 (M0+M1 Editor Core) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or native executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 LumeLog M0+M1：Tauri v2 桌面应用中可打开/保存多编码文本、正则搜索替换、超大文件只读流式查看的编辑器核心。

**Architecture:** 重逻辑全在纯 Rust crate `lumelog-core`（行索引/编码/取块解码/搜索/保存），Tauri 层只做命令转发与 IPC 流；前端 TypeScript 无框架，编辑模式用 CodeMirror 6，大文件用自研虚拟列表。`lumelog-can` 本计划只留空 crate 占位（Plan 2 填充）。

**Tech Stack:** Rust 2021 + memchr + encoding_rs + chardetng + regex + thiserror；Tauri v2 + Vite + TypeScript + CodeMirror 6；criterion；GitHub Actions（三平台）。

**Spec:** `docs/specs/2026-10-24-lumelog-v1-design.md`（Draft v0.6）。本计划覆盖 spec §1–§6、§8–§9（部分）、§12 M0/M1、§13.2（文本部分）。CAN（§7）、崩溃恢复与外部监听（§4.4 后两条）、行过滤（§6.4）属 Plan 2/3。

## Global Constraints

- 单文件上限 4GB：`LineIndex` 偏移用 `u32`；`size > 4GB` 拒绝打开（spec §4.1/A9）。
- 编辑阈值 200MB：`≤200MB` 编辑模式，`>200MB` 只读虚拟列表（spec §4.3/A1）。
- 性能门槛：P2 首屏 ≤500ms、P3 1GB 索引 ≤3s 且内存增量 ≤400MB、P4 取块+解码 ≤16ms(P95)、P5 1GB 字面量搜索 ≤10s（spec §2）。
- 编码显示名固定：`UTF-8`、`UTF-8 BOM`、`UTF-16 LE`、`UTF-16 BE`、`GBK`、`GB18030`、`BIG5`、`Shift-JIS`、`ISO-8859-1`；GB2312 按 GBK 解码；检测兜底 GB18030（spec §5）。
- IPC 错误契约：`{ code, message, detail? }`，code 为小写蛇形枚举串（spec §9）。
- 前端不使用任何框架（原生 DOM + TS）；CodeMirror 6（spec §8）。
- Git：每次 commit 后直接 `git push origin main`；commit 前先追加根目录 `CHANGELOG.md` 条目（claude.md 约定）。本 harness 的 shell 会剥引号，提交消息用无空格连写（如 `feat:lumelog-core-line-index`）。
- 依赖版本：`tauri = "2"`、`encoding_rs`/`memchr`/`regex`/`chardetng`/`thiserror`/`notify` 取当前 stable 最新版；Rust toolchain ≥1.79。

## Review Focus

spec 隐含但单任务测试不会天然覆盖、最容易伤到使用者的输入类，各由一个任务落钉：

1. 跨块边界的多字节字符：任意切分点解码结果必须与整体解码逐字符一致（Task 4 测试钉住）。
2. GB18030 四字节序列尾字节恰为 `0x0A` 造成误切行：展示行合并重试，搜索不受影响（Task 2/3 测试钉住）。
3. CRLF/混合行尾：`CR` 不独立成行、保存按打开时主导风格写回（Task 2/6 测试钉住）。
4. 正则命中跨越搜索块边界：重叠区去重不漏不重（Task 7 测试钉住）。
5. UTF-16 文件行扫描落在双字节中间：偶数对齐校验（Task 2 `build_aligned` 测试钉住）。

---

### Task 1: Workspace 脚手架

**Files:**
- Create: `Cargo.toml`（workspace root）, `crates/lumelog-core/Cargo.toml`, `crates/lumelog-core/src/lib.rs`, `crates/lumelog-can/Cargo.toml`, `crates/lumelog-can/src/lib.rs`（空占位）
- Create: `.gitignore`, `CHANGELOG.md`（首条）

**Interfaces:**
- Produces: workspace 成员 `lumelog-core`；后续所有 Rust 任务在此 crate 内新增模块并在 `lib.rs` 挂 `pub mod`。

- [ ] **Step 1:** 建 workspace root `Cargo.toml`：`[workspace] members = ["crates/lumelog-core", "crates/lumelog-can"]`，`resolver = "2"`。`lumelog-core` 依赖：memchr, encoding_rs, chardetng, regex, thiserror, criterion(dev, `[[bench]]` harness=false)。`lumelog-can` 空 lib。
- [ ] **Step 2:** `.gitignore`：`/target`, `node_modules`, `dist`, `*.lumelogtmp`。
- [ ] **Step 3:** `crates/lumelog-core/src/lib.rs` 加占位测试 `#[test] fn crate_links() {}`；Run `cargo test -p lumelog-core`，Expected: 1 passed。
- [ ] **Step 4:** commit `feat:workspace-scaffold` + push。

### Task 2: LineIndex（行索引）

**Files:**
- Create: `crates/lumelog-core/src/doc/line_index.rs`（`pub mod doc;` 挂入 lib.rs）
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub struct LineIndex { offsets: Vec<u32>, file_size: u64 }`，行 n 的字节区间 = `[offsets[n], offsets[n+1] or file_size)`（含 `\n`）。
  - `pub fn build(data: &[u8]) -> Result<LineIndex, CoreError>`（内存整体版，测试与 CLI 用）
  - `pub fn build_aligned(data: &[u8], pattern: u16 /*0x000A|0x0A00*/, little_endian: bool) -> Result<LineIndex, CoreError>`（UTF-16 行扫描：命中位置校验偶数对齐——Review Focus 5）
  - `pub struct LineIndexBuilder { pub fn new(file_size: u64) -> Self; pub fn push_chunk(&mut self, bytes: &[u8]); pub fn finish(self) -> Result<LineIndex, CoreError> }`（流式版，8MB 喂入，进度回调由调用方包）
  - `impl LineIndex { pub fn line_count(&self) -> usize; pub fn line_range(&self, n: usize) -> Option<(u64, u64)>; }`
  - `CoreError::TooLarge`（>u32::MAX 偏移时由 build/finish 返回）。
- 语义钉死：空文件 `line_count()==1` 且 `line_range(0)==(0,0)`；末行无 `\n` 计一行；`\r\n` 归属上一行（仅 `0x0A` 分割）。

- [ ] **Step 1: 写失败测试**（`doc/line_index.rs` tests）：`empty_file_is_one_line`、`no_trailing_newline_counts_last_line`、`crlf_belongs_to_previous_line`（输入 `b"ab\r\ncd"` → 2 行，第 0 行含 `\r\n`）、`utf16le_scan_even_aligned_only`（`0x0A` 落在奇字节的 UTF-16LE 语料，`build_aligned` 不把中间字节当换行——Review Focus 5）、`builder_chunks_equal_whole`（同一输入按 1/3/8MB 切分喂 builder 与 build 结果 offsets 相等）、`gb18030_four_byte_lf_missplit_is_indexable`（含尾字节 0x0A 的四字节序列，索引不 panic，行数可为 +1——本层只保证可索引）。
- [ ] **Step 2:** Run `cargo test -p lumelog-core doc::line_index`，Expected: FAIL（类型未定义）。
- [ ] **Step 3:** 实现。build 即 `memchr::memchr_iter(0x0A, data)` 收集 `pos+1` 为下一行起点；builder 维护跨块累计偏移。文件 >4GB 的检查在调用方（Task 6 `open`）与本层 `TooLarge` 双重把守。
- [ ] **Step 4:** Run 测试，Expected: PASS。commit `feat:lumelog-core-line-index` + push（含 CHANGELOG 条目）。

### Task 3: 编码模块（检测/解码/编码）

**Files:**
- Create: `crates/lumelog-core/src/encoding/mod.rs`
- Test: 同文件 tests + `crates/lumelog-core/tests/encoding_proptest.rs`

**Interfaces:**
- Produces:
  - `pub enum EncodingLabel { Utf8, Utf8Bom, Utf16Le, Utf16Be, Gbk, Gb18030, Big5, ShiftJis, Iso88591 }`（`as_str()` 返回 Global Constraints 固定显示名；`from_label("GBK")` 解析）
  - `pub fn detect(head: &[u8]) -> EncodingLabel`（顺序：BOM → 前 64KB 严格 UTF-8 → chardetng 置信度 ≥0.7 → 兜底 Gb18030；spec §5.2）
  - `pub fn decode_all(label: EncodingLabel, bytes: &[u8]) -> Decoded { text: String, invalid_count: usize }`
  - `pub struct IncrementalDecoder;`：`new(label) -> Result<Self>`、`feed(&mut self, bytes: &[u8]) -> DecodeSegment { text: String, invalid_count: usize }`、`finish(&mut self) -> usize`（尾字节 pending 由 encoding_rs 内部处理）
  - `pub fn encode_all(label, text: &str) -> Encoded { bytes: Vec<u8>, replace_count: usize }`（UTF-8 BOM 写出带 BOM；UTF-16 带 BOM）
- 约束：`Gbk`/`Gb18030` 直接用 `encoding_rs::GBK/GB18030`；GB2312 不在枚举（入口按 GBK 映射，spec §5.1）。

- [ ] **Step 1: 写失败测试**：`detect_utf8_strict_beats_chardetng`、`detect_falls_back_to_gb18030`（乱码字节）、`utf16le_bom_detection`、`gbk_roundtrip`（"台架HIL测试💩" encode→decode，replace_count 记录 emoji 不可编码）、`incremental_split_anywhere`（proptest：固定 GBK 语料，任意 1–3 个切分点 feed 拼接 == decode_all 的 text 与 invalid_count 之和）——Review Focus 1。
- [ ] **Step 2:** Run `cargo test -p lumelog-core`，Expected: encoding FAIL。
- [ ] **Step 3:** 实现：label↔encoding_rs 映射表驱动；detect 的 UTF-8 严格校验用 `std::str::from_utf8`；chardetng 结果映射表（GBK/GB18030/BIG5/Shift-JIS/windows-1252→Iso88591，其余→兜底）。
- [ ] **Step 4:** Run 测试含 proptest（1000 cases），Expected: PASS。commit `feat:lumelog-core-encoding` + push。

### Task 4: DocFile 取块解码 + char_map

**Files:**
- Create: `crates/lumelog-core/src/doc/file.rs`, `crates/lumelog-core/src/doc/chunk.rs`, `crates/lumelog-core/src/doc/mod.rs`（re-export）
- Modify: `lib.rs`（已有 `pub mod doc;`）
- Test: `crates/lumelog-core/tests/chunk_proptest.rs`

**Interfaces:**
- Consumes: Task 2 `LineIndex`、Task 3 `EncodingLabel/IncrementalDecoder`。
- Produces:
  - `pub struct DocFile`（持有 `File` + `LineIndex` + `EncodingLabel` + 路径 + `mtime/size` 元数据；`open(path, size_limit: u64) -> Result<DocFile, CoreError>`，>size_limit 返回 `CoreError::TooLarge`）
  - `impl DocFile { pub fn set_encoding(&mut self, label: EncodingLabel); pub fn line_count(&self) -> usize; pub fn chunk_at(&mut self, line_start: usize, line_count: usize) -> Result<Chunk, CoreError>; }`（内部 64MB LRU：`lru` 不引，手写 `VecDeque<(u64, Chunk)>` 按累计字节 ≤64MB 逐出）
  - `pub struct Chunk { pub start_line: usize, pub text: String, pub invalid_count: usize, pub char_map: CharMap }`
  - `pub struct CharMap`：`from_text(start_line: u32, text: &str, index: &LineIndex, label: EncodingLabel) -> CharMap`（对块首字节起做字节↔cu16 步进换算；UTF-16 步长 2 对齐——Review Focus 5）；`pub fn byte_to_line_cu(&self, abs_byte: u64) -> (usize /*line*/, u32 /*cu16 in line*/)`。实现取码提示：由行边界对齐的解码文本 + 每行 cu16 长度即可换算行内偏移，绝对字节→(行,行内字节) 用 LineIndex，行内字节→cu16 用当前编码步进规则（UTF-8 按序列长度、GBK/GB18030 首字节判长、UTF-16 半区累计）——此为签名+测试决定的算法，允许实现细节自选，测试钉结果。
  - `pub enum CoreError { Io(std::io::Error), TooLarge, LineOutOfRange, Encoding(String) }`（thiserror；Task 3 之前引用，实际定义放 `doc/mod.rs` 顶部 `error.rs` 亦可，Task 2 起使用）。

- [ ] **Step 1: 写失败测试**：`chunk_first_two_lines_utf8`、`chunk_gb18030_mismerged_line_retries_next_line`（块级：Task 2 语料中误切行解码 invalid_count>0 且合并后 U+FFFD 消失）、`set_encoding_keeps_line_numbers`（同文件 GBK/UTF-8 标签切换行数不变——spec §5.2）、`charmap_all_alignments`（proptest：UTF-8/GBK 混合文本（含 emoji/4 字节 GB18030），块内随机字节位置（含 `\n` 前后）byte_to_line_cu 与"整体 decode 后按行 split 数 cu16"一致——Review Focus 1/5）。
- [ ] **Step 2:** Run，Expected: FAIL。
- [ ] **Step 3:** 实现 DocFile/Chunk/CharMap。读文件用 `std::fs::File` + `FileExt::read_at`（win）/ `pread`（unix）统一走 `std::os` 分支或直接 `seek+read`（简单，锁内串行即可，性能不足再优化——本任务允许 seek）。
- [ ] **Step 4:** Run 测试含 proptest，Expected: PASS。commit `feat:lumelog-core-docfile-chunk` + push。

### Task 5: 保存（原子写/EOL 主导/冲突检测/不可映射提示）

**Files:**
- Create: `crates/lumelog-core/src/doc/save.rs`
- Modify: `doc/file.rs`（`DocFile` 增加 `eol_style` 字段与记录）
- Test: save.rs 内 tests

**Interfaces:**
- Consumes: Task 3 `encode_all`。
- Produces:
  - `pub enum EolStyle { Lf, Crlf, Mixed }`；`pub fn detect_eol(text: &str) -> EolStyle`（CRLF 计数 ≥ LF 独立计数 → Crlf，任一 >0 且两类都有 → Mixed）
  - `pub struct SaveOptions { pub path: Option<PathBuf> /*None=原路径*/, pub encoding: EncodingLabel, pub eol: EolStyle /*写入用，Mixed 视为 Crlf*/, pub overwrite_policy: OverwritePolicy }`；`OverwritePolicy::{ SafetyConfirm(以 disk_meta 比对触发冲突询问), Force }`
  - `pub fn save_text(doc: &mut DocFile, text: &str, opts: &SaveOptions, disk_meta: (u64, SystemTime)) -> Result<SaveReport, CoreError>`；`SaveReport { replace_count: usize, conflict: bool }`——conflict=true 且 policy=SafetyConfirm 时**不落盘**返回 `CoreError::Conflict(disk_meta_now)`，由 UI 层决定重调 Force。replace_count>0 时同样先回 `CoreError::Unmappable(usize)`（policy 不含 Force 时），保证 spec §4.4 "确认后执行"。
  - 原子写：同目录 `.{stem}.lumelogtmp` → write+fsync → `fs::rename`；rename 失败且 Windows 错误为权限类 → `CoreError::Locked`（降级逻辑在 UI 层——spec §4.4）。
  - 行尾转换：落盘字节 = text 先归一 LF 再按 opts.eol 展开。

- [ ] **Step 1: 写失败测试**：`save_roundtrip_gbk_crlf`（含中文文本 → GBK+Crlf 保存 → 重开 decode 一致，EOL 逐行验证——Review Focus 3）、`detect_eol_mixed`、`conflict_blocks_save_without_force`、`unmappable_reports_count_before_write`（emoji→GBK 返回 Unmappable(1) 且磁盘未变）、`atomic_write_leaves_no_tmp_on_success`。
- [ ] **Step 2:** Run，Expected: FAIL。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** Run，Expected: PASS。commit `feat:lumelog-core-save` + push。

### Task 6: 搜索（分块正则 + 重叠去重 + 取消）

**Files:**
- Create: `crates/lumelog-core/src/search.rs`
- Test: 同文件 tests

**Interfaces:**
- Consumes: Task 2 `LineIndex`（命中字节→行号换算）、Task 4 `DocFile`（块读）。
- Produces:
  - `pub struct SearchQuery { pub pattern_text: String, pub mode: Mode /* Literal | Regex */, pub encoding: EncodingLabel /* 查询串按此编码 encode——spec §6.2/A8 */, pub case_sensitive: bool }`
  - `pub struct Match { pub line: u32, pub byte_in_line: u64, pub len: usize /*字节长*/, pub snippet: String /*解码后 ±40 字符*/ }`
  - `pub fn search_stream(doc: &mut DocFile, q: &SearchQuery, on_match: impl FnMut(Match), progress: impl FnMut(u64 /*indexed_bytes_done*/) -> bool /*false=取消*/ ) -> Result<SearchStats, CoreError>`
  - 算法钉死：8MB 窗口 `regex::Regex`（`boring/unicode` 默认）逐块 `find_iter`；Literal 用 `regex::escape`；窗口重叠 `OVERLAP = 1024` 字节；跨重叠区重复命中按 `(abs_byte_start)` 去重；行号 = `partition_point(index.offsets, hit_start)`−1；snippet 取所在行解码文本前后 40 cu16。
  - 查询串 encode 后若含 `0x00`（UTF-16 文件场景）：v1 明确返回 `CoreError::Encoding("UTF-16 search unsupported in v1")`【本计划决定：UTF-16 大文件搜索进 Plan 3，编辑模式 UTF-16 小文件走前端搜索】。

- [ ] **Step 1: 写失败测试**：`literal_hits_gbk_chinese`（GBK 语料搜"错误"）、`regex_overlap_match_not_missed_not_doubled`（构造恰跨 8MB 边界+1KB 重叠区的命中，出现 1 次——Review Focus 4）、`cancel_stops_early`（progress 第 2 次返回 false → stats.cancelled=true 且命中数 < 全量）、`line_number_correct`、`stats_counts_all_matches_gb18030_file`。
- [ ] **Step 2:** Run，Expected: FAIL。
- [ ] **Step 3:** 实现 `search.rs`。
- [ ] **Step 4:** Run，Expected: PASS。
- [ ] **Step 5: bench**：`crates/lumelog-core/bench/search.rs` criterion：合成 1GB 固定种子日志（`tests_common::gen_log(size_mb)` 工具函数放 `crates/lumelog-core/src/tests_common.rs`，`#[cfg(test)]`+bench 双挂），`search_literal_1gb` benchmark 打印耗时。手动 Run `cargo bench -p lumelog-core -- search_literal_1gb`，Expected: 输出耗时 < 10s（P5，记录进 CHANGELOG；不卡 CI）。commit `feat:lumelog-core-search` + push。

### Task 7: lumelog CLI 冒烟（M0 验收）

**Files:**
- Create: `crates/lumelog-core/src/bin/lumelog.rs`
- Test: `crates/lumelog-core/tests/cli_smoke.rs`

**Interfaces:**
- Consumes: Task 2–6 全部公开接口。
- Produces: `lumelog <open|search|save> --file PATH [--encoding LABEL] [--pattern STR] [--line-count|--head N]`；`open` 打印 `{size, lines, encoding(detected), eol}` JSON；无参数退出码 2。

- [ ] **Step 1:** 实现 bin（clap 不引，手工 `std::env::args` 解析即可——避免多一个依赖）。
- [ ] **Step 2:** 集成测试 `cli_smoke.rs`：用 `assert_cmd` 或 `std::process::Command` 跑 `cargo run -p lumelog-core --bin lumelog -- open --file tests/data/sample_gbk.log`，断言 JSON lines/encoding 字段。Expected 先 FAIL（数据文件不存在）→ 建 `tests/data/sample_gbk.log`（GBK 编码 20 行台架日志）。
- [ ] **Step 3:** Run `cargo test -p lumelog-core --test cli_smoke`，Expected: PASS。
- [ ] **Step 4:** 手动 P2/P3 冒烟：`lumelog open --file <1GB 合成日志>` 计时（CLI 路径：首屏=立即返回 JSON，索引在内存整体 build——记录耗时进 CHANGELOG 作 P3 参考，UI 级验收在 Task 11）。commit `feat:lumelog-cli-smoke` + push。

### Task 8: Tauri v2 壳 + IPC 命令层

**Files:**
- Create: `app/src-tauri/`（`cargo create-tauri-app` 生成后改造：`Cargo.toml` 改名 `lumelog-app`，`commands.rs`, `state.rs`, `lib.rs` 挂命令）
- Create: `app/frontend/`（Vite vanilla-ts 骨架）
- Modify: workspace root `Cargo.toml` members 增 `"app/src-tauri"`

**Interfaces:**
- Consumes: lumelog-core 全接口。
- Produces（与 spec §9 对齐，命名即前端调用名）:
  - `doc_open(path) -> DocInfo { doc_id, size, mtime_millis, encoding, line_count_estimate, total_lines: Option<u64> }`；索引后台线程逐 8MB 推进，进度经 channel `on_index_progress { doc_id, indexed_bytes, done }`
  - `doc_chunk(doc_id, line_start, line_count, on_data: Channel<ChunkPayload { start_line, text, invalid_count }>)`——char_map 不出 IPC（前端只渲染文本；行内定位 Plan 3 搜索 UI 再传，【本计划决定】）
  - `doc_save(doc_id, text, path?, encoding, eol, force) -> Result<SaveReport, IpcError>`；错误码枚举：`io|too_large|conflict|unmappable|locked|line_out_of_range`（`IpcError { code, message, detail? }` serde）
  - `search_run(doc_id, pattern, mode, case_sensitive, on_batch: Channel<Vec<MatchJson { line, byte_in_line, len, snippet }>>, on_done: Channel<SearchStatsJson>)`；取消：`search_cancel(request_id)`；`CoreError::Encoding(UTF-16)` → code `encoding_unsupported`
  - `doc_list()`（前端重开恢复用）、`app_ping() -> "pong"`（首屏可交互探针，P1 计时终点）
- 状态：`state.rs` 的 `AppDocs { docs: Mutex<HashMap<u64, Arc<Mutex<DocFile>>>> }` 挂 `tauri::manage`。

- [ ] **Step 1:** `npm create tauri-app@latest app -- --template vanilla-ts --manager npm`（非交互参数按其 CLI 现状调整）；workspace 收编；`cargo check -p lumelog-app` Expected: PASS（默认模板）。
- [ ] **Step 2:** 实现 `state.rs`/`commands.rs` 上述签名（Tauri v2 `Channel<T>` 参数）；`doc_open` 的索引线程：`std::thread::spawn` + LineIndexBuilder，边推进 clone 进度到 Channel。测试文件：Rust 侧 `#[cfg(test)]` 用 `tauri::test` 重（起 App 需配置）——**本任务用纯逻辑拆分保可测**：`build_index_with_progress(file, chunk=8MB, on_progress) -> LineIndex` 放 core（Task 2 已有 builder，包一层），commands 只做转发；Step 2 的验证 = `cargo check -p lumelog-app` 通过 + Task 9/10 的 E2E。
- [ ] **Step 3:** `doc_open` 前置校验（spec §9：不存在/目录/无权限/ >4GB → 对应 code），单测转纯函数 `validate_open_path() -> Result<(), IpcError>` 并 `#[test]`。
- [ ] **Step 4:** `npm run tauri dev` 手工冒烟：窗口出现，前端 console `invoke("app_ping")` 返回 pong。commit `feat:tauri-shell-ipc` + push。

### Task 9: 前端编辑模式（CodeMirror 6 + Tab + 状态栏）

**Files:**
- Create: `app/frontend/src/{main.ts, ipc.ts, editor.ts, tabs.ts, statusbar.ts, encoding-menu.ts}`
- Modify: `app/frontend/index.html`, `vite.config.ts`（tauri dev server 端口固定 1420）

**Interfaces:**
- Consumes: Task 8 IPC（`doc_open/doc_chunk/doc_save/search_*`）。`ipc.ts` 导出 `invoke<CamelName>` 包装与 Channel 订阅函数（签名同 Task 8 Produces）。
- Produces: 可用的窗口行为：打开（对话框 + 拖放）→ ≤200MB 进 CodeMirror 编辑，Ctrl+S 保存（冲突/不可映射弹窗三选），编码下拉（切换即 `doc_open` 重声明标签 + 重取全文重渲染），EOL 下拉，dirty 圆点，最近 20 文件菜单。搜索：CodeMirror search 面板（前端正则，逐个/全部替换）+ Ctrl+Shift+F 调 `search_run` 只读列中定位（Plan 3 完善联动，本任务仅展示命中列表与 snippet）。语法高亮（spec §8）：默认 PlainText + log 风格（时间戳/级别/数字/hex 着色，`@codemirror/language` 的 `StreamLanguage` 简易 mode）+ JSON/INI/C（lezer 包），扩展名推断 + 状态栏手动切换。

- [ ] **Step 1:** `npm i @codemirror/{state,view,commands,search,language,lang-json,lang-ini,lang-c}` `@tauri-apps/api @tauri-apps/plugin-dialog`；vite 构建基线 `npm run build` PASS。
- [ ] **Step 2:** `editor.ts`：`openEditable(doc)` 拉全块文本（`doc_chunk` 循环拼接）建 EditorView；`tabs.ts`/`statusbar.ts`/`encoding-menu.ts` 按 Produces 行为实现。无单测框架要求：以 `npx vitest run` 跑 `ipc.ts` 纯逻辑（错误码→中文文案映射表）`ipc.test.ts`（mock Channel）。
- [ ] **Step 3:** vitest 用例：`ipc_error_code_maps_to_zh_text`（8 个 code 全覆盖）、`encoding_menu_labels_match_spec`（9 个显示名逐个断言——Global Constraints 钉死值）。Run `npx vitest run`，Expected: PASS。
- [ ] **Step 4:** `npm run tauri dev` 手测清单：打开 GBK 台架日志不乱码/保存往返/切编码保留滚动/搜索替换。commit `feat:frontend-edit-mode` + push。

### Task 10: 前端只读虚拟列表（大文件模式）

**Files:**
- Create: `app/frontend/src/listview.ts`
- Modify: `main.ts`（>200MB 路由到 ListView）, `app/src-tauri/src/commands.rs`（`doc_chunk` 已就绪，补 `doc_open` 返回 mode 字段）

**Interfaces:**
- Produces: `class ListView { constructor(host: HTMLElement, docId: number, totalLines: () => number); scrollToLine(n: number): void }`——固定行高 20px，可见 ±200 行窗口，每窗口请求一次 `doc_chunk`，行号 gutter 原始值；索引未达的行显示"待索引"占位，到达后增量刷新（订阅 `on_index_progress`）。

- [ ] **Step 1:** 实现 listview.ts（原生 DOM，绝对定位行 + 二分滚动映射）。
- [ ] **Step 2:** 手测：`npm run tauri dev` 打开 Task 7 的 1GB 合成日志：首屏显示、滚动流畅、索引进度条走完后可 `scrollToLine` 到底部（Ctrl+G 输入行号入口）。
- [ ] **Step 3:** P4 后端侧核：在 core bench 加 `chunk_at_gb`（随机 1000 次取块计时 P95），Run `cargo bench -- chunk_at_gb`，Expected: <16ms 记录进 CHANGELOG。commit `feat:frontend-readonly-listview` + push。

### Task 11: M1 集成验收 + CI + 发布物

**Files:**
- Create: `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `docs/test-checklist.md`, `CHANGELOG.md`（M1 汇总条目）
- Modify: `tauri.conf.json`（productName/bundle: nsis,msi,dmg,deb/AppImage）

- [ ] **Step 1:** `docs/test-checklist.md`：三平台 × {1GB 日志（P2/P3/P4/P5 实测记录表）、GBK/GB18030/UTF-16 样例、CRLF 保存、搜索取消、冲突弹窗}，逐项命令与预期。
- [ ] **Step 2:** ci.yml：三平台 matrix `cargo test --workspace` + `cargo clippy -D warnings` + 前端 `npm run build && npx vitest run`；bench 不进 CI（手动 job_dispatch）。release.yml：tag 触发三平台打包传 gitee releases（`gitee-release` action 或 curl API，token 用 secrets）。
- [ ] **Step 3:** 本地跑 `npm run tauri build`（Windows）Expected: NSIS 安装包产出，体积 ≤15MB（P6 记录）。
- [ ] **Step 4:** 按 checklist 在 Windows 实测 P1–P5b 并填表（macOS/Linux 由 CI 产物 + 后续手动补测，不阻塞本计划收尾）。若 P1 超标，做 tauri 资源预载优化后再收尾。
- [ ] **Step 5:** commit `chore:m1-acceptance-ci` + push；更新 claude.md 功能清单勾除已交付项。
