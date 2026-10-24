# LumeLog Plan 2 (M2 CAN Engine) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or native executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 LumeLog M2 的纯 Rust CAN 引擎（`lumelog-can`）：DBC 子集解析、Intel/Motorola 信号位序解码、candump/ASC/通用正则三种逐行流式日志源，以及一个可独立验收的 `lumelog-can` 解码 CLI。

**Architecture:** 全部逻辑在独立 crate `lumelog-can`（不依赖 Tauri，可 CLI 复用、可基准测试）。DBC 行式子集解析器产出 `Database`（messages/signals），信号解码把一帧 8–64 字节按 DBC 定义拆成物理值；三种日志源统一实现 `FrameSource` trait，把「(行号, 文本) 行流」逐行解码成 `Frame`（畸形行跳过并计数、绝不 panic）。**本计划只做引擎**：IPC 命令层与 CanPanel 报文表格（TanStack Table）留到 Plan 3，二者同为 M2 验收的另一半。

**Tech Stack:** Rust 2021（toolchain ≥1.79）+ thiserror + regex + proptest(dev)；无新重依赖（解码为纯位运算，CLI 输出手工格式化 JSON/CSV，不引 serde）。参考对拍：cantools（Python，仅开发期生成 golden 夹具，不进 CI）。

**Spec:** `docs/specs/2026-10-24-lumelog-v1-design.md`（§7 CAN 模块、§9 IPC 契约中的 CAN 命令、§12 M2 验收、§13.1 Rust 单元与对拍测试）。本计划覆盖 §7.1–§7.3 + §7.5 的「解码结果 CSV/JSONL 导出」；§7.4 CanPanel、§7.5「原始日志行导出 export_lines」、§9 `dbc_import`/`can_parse` IPC 属 Plan 3。

## Global Constraints

- 语言/工具链：Rust 2021，`rust-version = 1.79`；`edition`/`version` 走 workspace 继承。
- 依赖最小化：`lumelog-can` 仅用 `thiserror`（错误）、`regex`（通用正则源）；dev 依赖 `proptest = "1"`。**不引 serde/serde_json**——解码结果 JSONL/CSV 用 `format!` 手写；如需结构化序列化，留给 Plan 3 的 IPC 层统一处理。
- crate 边界：`lumelog-can` 不得依赖 `lumelog-core`、不得依赖 Tauri。日志源只消费抽象行流 `Iterator<Item=(u64 /*行号*/, String /*文本*/)>`，与 `DocFile` 的对接在 Plan 3。
- 逐行流式 + 健壮性：任一源遇到畸形/不支持行**跳过并计数**，绝不 panic、绝不中断整条流（spec §7.3）。
- 位序算法以 cantools 为准（spec §7.2/§13.1）：Intel=`@1`（LittleEndian）、Motorola=`@0`（BigEndian/DBC 斜线位序）。实现必须逐位匹配 `tests/data/golden_bitorder.json`（由 dev 脚本 `gen_golden.py` 用 cantools 生成并提交）；未通过该夹具前不得声称位序完成。
- DBC 扩展帧：`BO_` 的 `<id>` 若置位 bit31（`id >= 0x8000_0000`）视为扩展帧，`is_extended=true`，`can_id = id & 0x1FFF_FFFF`。
- 复用（muxed）信号：`M`/`m<n>` 解析存储；解码只输出「当前复用值」下的非复用信号 + 匹配复用值的信号（spec §7.1）。
- IPC 错误契约（前瞻对齐 Plan 3）：`CanError` 各变体须可映射为 `{ code, message, detail? }`，code 小写蛇形（与 core 的 IpcError 同风格），本计划内以 `#[test]` 钉住 `CanError::ipc_code()` 字符串稳定。
- Git：每次 commit 后直接 `git push origin <当前分支>`；commit 前先追加根目录 `CHANGELOG.md` 条目（claude.md 约定）。本 harness 的 shell 会剥引号，提交消息用无空格连写（如 `feat:can-dbc-parser`）。
- 里程碑收尾（M2 引擎验收）：Task 7 用 10 万帧合成 candump 语料跑解码，行数/ID/数据逐帧与生成端一致（正确性面），`decode` 吞吐 bench 记录进 CHANGELOG（对齐 §12「10 万帧真实台架日志解码正确」的引擎侧可测部分）。

## Review Focus

spec 隐含但单任务测试不会天然覆盖、最容易伤到使用者的输入类，各由一个任务落钉：

1. **Motorola `@0` 起始位与斜线位序**：DBC 的 MSB 起始 + 「字节内向上、跨字节向下」锯齿位序最易整体位移一位或跨字节错位；以 cantools golden 逐信号对拍钉死（Task 3）。
2. **有符号补码 + factor/offset 浮点**：负 raw（最高位为 1）按二进制补码还原，`physical = raw*factor + offset` 的浮点往返在 factor=0.1 类非精确系数下按容差比对（Task 3）。
3. **FD / RTR / error-frame 行**：candump `id#R`(RTR)、`id##<flags><data>`(FD，dlc 可到 64)、`id#ErrorFrame`(不支持→跳过计数)，ASC 的 `boe`/`bcs`/`Rx r`(remote)——不得污染后续行、不得 panic（Task 4/5）。
4. **DBC 语法鲁棒性**：坏行「跳过继续」并回传行号+上下文；`CM_`/`BA_DEF_`/`BA_`/`VAL_TABLE_` 与真正的 `BO_`/`SG_` 不混淆；`BO_` 行 dlc 与节点间无逗号（Task 2）。
5. **通用正则源命名组缺失/畸形**：用户正则缺 `id`/`data` 命名组 → 构建期即报错；行不匹配 → 跳过计数；`data` 十六进制长度与 dlc 不一致 → 按实际 hex 长度定 dlc（Task 6）。

---

### Task 1: lumelog-can 数据模型 + 错误 + workspace 收编

**Files:**
- Modify: `Cargo.toml`（workspace deps 增 `proptest = "1"`）
- Modify: `crates/lumelog-can/Cargo.toml`（deps：`thiserror.workspace`、`regex.workspace`；dev-deps：`proptest`）
- Create: `crates/lumelog-can/src/lib.rs`（本任务先 `pub mod {error, dbc, frame};`——`decode` 由 Task 3、`source` 由 Task 4 各自挂入；+ `crate_links` 占位测试）
- Create: `crates/lumelog-can/src/error.rs`
- Create: `crates/lumelog-can/src/dbc.rs`（仅数据模型 struct + enum，解析器在 Task 2）
- Create: `crates/lumelog-can/src/frame.rs`（`Frame`/`FrameFlags`/`DecodedFrame`/`SignalValue`/`SourceStats`）

**Interfaces:**
- Produces:
  - `pub enum CanError { Io(std::io::Error), DbcSyntax { line: usize, context: String, kind: String }, BadRegex(String), RegexCompile(String), SourceFormat(String) }`（thiserror）；`impl CanError { pub fn ipc_code(&self) -> &'static str }` → 依次 `"io" | "dbc_syntax" | "bad_regex" | "regex_compile" | "source_format"`。
  - `pub enum ByteOrder { LittleEndian, BigEndian }`（LittleEndian=Intel `@1`，BigEndian=Motorola `@0`）
  - `pub enum Mux { NonMuxed, Selector, Indexed(u8) }`（`M`→Selector，`m<n>`→Indexed(n)，普通信号 NonMuxed）
  - `pub struct Signal { pub name: String, pub start_bit: u32, pub length: u32, pub byte_order: ByteOrder, pub is_signed: bool, pub factor: f64, pub offset: f64, pub minimum: f64, pub maximum: f64, pub unit: String, pub receivers: Vec<String>, pub value_table: Vec<(i64, String)>, pub mux: Mux }`
  - `pub struct Message { pub id: u32, pub name: String, pub dlc: u8, pub transmitter: String, pub is_extended: bool, pub signals: Vec<Signal> }`
  - `pub struct Database { pub version: String, pub nodes: Vec<String>, pub messages: Vec<Message> }`
  - `pub struct FrameFlags { pub is_fd: bool, pub brs: bool, pub esi: bool, pub is_remote: bool, pub is_error_frame: bool }`
  - `pub struct Frame { pub line_number: u64, pub timestamp: Option<f64>, pub channel: u8, pub can_id: u32, pub dlc: u8, pub data: [u8; 64], pub flags: FrameFlags }`
  - `pub struct SignalValue { pub name: String, pub raw: i64, pub physical: f64, pub enum_text: Option<String>, pub unit: String }`
  - `pub struct DecodedFrame { pub frame: Frame, pub message_name: Option<String>, pub signals: Vec<SignalValue> }`
  - `pub struct SourceStats { pub decoded: u64, pub skipped: u64 }`

- [ ] **Step 1:** 写 workspace deps + crate Cargo.toml 收编（本 crate 目前无任何依赖）。
- [ ] **Step 2:** 在 `frame.rs`、`dbc.rs`、`error.rs` 落地上述类型/枚举（本任务不写算法，只定接口，让后续任务有稳定签名）。`CanError` 用 `#[derive(thiserror::Error)]`，`ipc_code()` 手写匹配。
- [ ] **Step 3:** `lib.rs` 加 `#[cfg(test)] #[test] fn ipc_codes_are_stable()`：对五个 `CanError` 变体断言 `ipc_code()` 返回约定字符串（钉住 spec §9 code 契约）；`crate_links()` 空测试保证编译。
- [ ] **Step 4:** Run `cargo test -p lumelog-can`，Expected: 2 passed。Run `cargo clippy -p lumelog-can -- -D warnings`，Expected: 净（允许 `#[allow(dead_code)]` 过渡，Task 2+ 消化）。
- [ ] **Step 5:** CHANGELOG 追加条目 → commit `feat:can-model-workspace` + push。

### Task 2: DBC 子集解析器

**Files:**
- Create: `crates/lumelog-can/src/dbc/parse.rs`（把 `dbc.rs` 移为 `dbc/mod.rs` 并 `pub mod parse;` re-export）
- Create: `crates/lumelog-can/tests/data/sample.dbc`（Vector 风格：≥2 条 BO_/SG_，含 Intel+Motorola、有符号、mux `M`/`m1`、`VAL_` 枚举、`CM_` 注释、`BA_DEF_`/`BA_` 属性、≥1 条故意坏行）
- Test: `crates/lumelog-can/tests/dbc_golden.rs`

**Interfaces:**
- Consumes: Task 1 `Database/Message/Signal/ByteOrder/Mux`。
- Produces:
  - `pub struct ParseOutcome { pub database: Database, pub errors: Vec<CanError> }`
  - `pub fn parse(src: &str) -> Result<Database, CanError>`（严格：首个语法错即返回 `CanError::DbcSyntax{line,context,kind}`）
  - `pub fn parse_lenient(src: &str) -> ParseOutcome`（逐行解析，坏行记 `DbcSyntax` 进 errors 并跳过——spec §7.1「跳过坏行继续」模式）
  - 解析覆盖：`VERSION`/`NS_`/`BS_:`/`BU_`(节点名)；`BO_ <id> <name>: <dlc> <node>`（dlc 与 node 间**无逗号**，以真实 Vector 导出为准；`id` bit31→is_extended，spec §7.1 + Global Constraints）；`SG_ <name> : <|>|<start>|<len>@<order><sign> (<factor>,<offset>) [<min>|<max>] "<unit>" <receivers>`；`VAL_ <msg> <sig> <a> "<b>" ...` → 填 `Signal::value_table`；`VAL_TABLE_` 解析存储但不强制使用；`CM_`/`BA_DEF_`/`BA_` 跳过不实现语义（spec §7.1/A5）。

- [ ] **Step 1:** 写失败测试 `dbc_golden.rs`：`parses_sample_messages_and_signals`（对 sample.dbc 断言 message 数、首个 message 的 id/dlc/扩展位、含一条 `ByteOrder::BigEndian` 有符号信号的 start_bit/length/factor/offset 精确值）、`skips_comments_and_attributes`（CM_/BA_ 不产生 message/signal）、`val_table_populates_enum`（`VAL_` 命中的 signal.value_table 含期望 `(a,"b")`）、`mux_marker_parsed`（`M`→`Mux::Selector`、`m1`→`Mux::Indexed(1)`）、`extended_id_sets_flag_and_masks`（`0x80000123` → is_extended=true, id=0x123）、`lenient_skips_bad_line_records_context`（sample.dbc 的坏行：`parse_lenient` 返回 database 可用且 errors 含该 line 号，`parse` 严格模式对该行报 `DbcSyntax`）——Review Focus 4。
- [ ] **Step 2:** Run `cargo test -p lumelog-can --test dbc_golden`，Expected: FAIL（`parse` 未定义）。
- [ ] **Step 3:** 实现 `parse.rs`：按行状态机（`BU_` 之后、`BO_` 之后累积其 `SG_` 直到下一个 `BO_` 或非缩进边界）。`SG_` 用一次正则或手工切分 `<|>|<start>|<len>@<order><sign>`；factor/offset/min/max 用 `f64::from_str`。坏行只记录不中断（lenient）。
- [ ] **Step 4:** Run `cargo test -p lumelog-can --test dbc_golden`，Expected: PASS。
- [ ] **Step 5:** CHANGELOG 条目 → commit `feat:can-dbc-parser` + push。

### Task 3: 信号位序解码（Intel/Motorola）——cantools 对拍定案

**Files:**
- Create: `crates/lumelog-can/src/decode.rs`
- Create: `crates/lumelog-can/tests/data/bitorder.dbc`（专供对拍：覆盖 Intel/Motorola × signed/unsigned × 跨字节 start_bit/length，含 factor≠1、offset≠0、负值）
- Create: `crates/lumelog-can/tests/data/golden_bitorder.json`（**由 cantools 生成的期望表**：每条 `{frame_hex, signal:"msg.sig", raw:int, physical:float}`）
- Create: `crates/lumelog-can/tests/gen_golden.py`（dev 脚本：读 bitorder.dbc + 固定帧 → 用 `cantools.database` 解码 → 写 golden_bitorder.json；**不在 CI 运行**，仅再生成用）
- Test: `crates/lumelog-can/tests/bitorder_golden.rs`, `crates/lumelog-can/tests/decode_proptest.rs`

**Interfaces:**
- Consumes: Task 1 `Signal/ByteOrder/Message/Database/SignalValue`。
- Produces:
  - `pub fn extract_raw(data: &[u8], sig: &Signal) -> u64`（按 `byte_order` 从 8–64 字节帧取 `length` 位；LittleEndian 以 start_bit 为 LSB 向高位；BigEndian 按 DBC Motorola 斜线位序以 start_bit 为 MSB——算法以 golden 对拍为准，见 Step 说明）
  - `pub fn raw_to_signed(raw: u64, length: u32) -> i64`（按 length 位二进制补码符号扩展——Review Focus 2）
  - `pub fn to_physical(raw_signed: i64, sig: &Signal) -> f64`（`(raw_signed as f64) * factor + offset`；factor/offset 为 f64，spec §7.2）
  - `pub fn decode_signals(msg: &Message, data: &[u8]) -> Vec<SignalValue>`（含 value_table 命中填 `enum_text`；muxed 信号本任务先全解码并保留，Task 复用过滤在 CLI/Plan 3 用 `Mux` 再筛；此处非复用 + muxed 都返回，测试钉非 muxed 精确值）

- [ ] **Step 1: 生成对拍夹具（前置）**。确保 `pip install cantools==39.4.5`（或当前 stable，版本写进 gen_golden.py 头注释）。Run `python crates/lumelog-can/tests/gen_golden.py`，产出 `tests/data/golden_bitorder.json`。若 cantools 不可安装（网络受限），**停下征询用户**：SDD 停条件之一（越出 worktree 的网络安装副作用）。夹具提交进仓库后，Rust 测试与 CI 不再需要 Python。
- [ ] **Step 2:** 写失败测试 `bitorder_golden.rs`：读 `tests/data/bitorder.dbc`（`parse`）与 `golden_bitorder.json`，逐条断言 `extract_raw`→(signed)→`to_physical` 与 golden 的 `raw` 精确相等、`physical` 在 `1e-9` 相对容差内相等。`decode_proptest.rs`：proptest 随机 (start_bit,length∈1..64,order,signed,factor,offset) + 随机 8 字节帧，断言 `to_physical(extract_raw)` 与「按定义直接手算该位段」一致（往返自洽，不依赖 cantools）——Review Focus 1/2。
- [ ] **Step 3:** Run `cargo test -p lumelog-can --test bitorder_golden`，Expected: FAIL（`extract_raw` 未定义）。
- [ ] **Step 4:** 实现 `extract_raw`：
  - LittleEndian（Intel）：`for k in 0..length { bit_index = start_bit + k; byte = bit_index/8; bit = bit_index%8; raw |= ((data[byte]>>bit)&1) << k }`。
  - BigEndian（Motorola，DBC 斜线位序）：start_bit 为 MSB 位置；给出「从 start_bit 起，先在同字节内向低位走、跨到下一字节」的标准 DBC 递推（写清注释），**以 Step 6 的 golden 对拍结果为最终判据**——若首版实现不匹配，据 golden 调整位序递推直到逐位相等。
- [ ] **Step 5:** 实现 `raw_to_signed`/`to_physical`/`decode_signals`（补码符号扩展 + factor/offset + value_table 命中）。
- [ ] **Step 6:** Run `cargo test -p lumelog-can`（含 bitorder_golden + decode_proptest），Expected: PASS。若 golden 不匹配 → 回到 Step 4 修 Motorola 递推，不得改 golden（golden 是权威）。
- [ ] **Step 7:** CHANGELOG 条目（记「位序算法经 cantools golden 对拍通过；cantools 版本 x.y」）→ commit `feat:can-signal-decode` + push。

### Task 4: FrameSource trait + candump ASCII 源

**Files:**
- Create: `crates/lumelog-can/src/source/mod.rs`（`FrameSource` trait + `Line` 别名 + 共享十六进制工具）
- Create: `crates/lumelog-can/src/source/candump.rs`
- Create: `crates/lumelog-can/tests/data/candump_basic.log`（含普通帧/带 `(ts)` 前缀/RTR `#R`/FD `##1..`/error frame/空行/畸形行）
- Test: `crates/lumelog-can/tests/candump.rs`

**Interfaces:**
- Consumes: Task 1 `Frame/FrameFlags/SourceStats`。
- Produces:
  - `pub type Line = (u64 /*行号*/, String /*文本*/)`
  - `pub trait FrameSource { fn next_frame(&mut self) -> Option<Frame>; fn stats(&self) -> SourceStats; }`
  - `pub struct CandumpSource<I: Iterator<Item = Line>> { ... }`；`impl<I: Iterator<Item=Line>> CandumpSource<I> { pub fn new(lines: I) -> Self }`；`impl FrameSource`。
  - 支持形态（spec §7.3.1）：`(1.234567) can0 123#DEADBEEF`、`can0 735#DEADBEEF`（无 ts→`timestamp=None`）、iface→channel（`can0`→0，解析数字后缀）；RTR `id#R`→`flags.is_remote=true`、data 空；FD `id##<flags_hex><data_hex>`→`is_fd=true` 且从 flags 位解析 BRS/ESI；长度 = `#` 后 hex 字符数/2；扩展 id（≤8 hex）。error-frame 形态与不匹配行 → 跳过 `stats.skipped += 1`。
  - 十六进制工具 `pub(crate) fn hex_to_bytes(s: &str) -> Option<Vec<u8>>`（奇数长度/非法字符 → None）。

- [ ] **Step 1:** 写失败测试 `candump.rs`：`parses_plain_and_ts_frames`（行号随输入递增、ts 解析为 f64、id/data/dlc 正确）、`channel_from_iface`（`can2`→channel 2）、`rtr_sets_remote_no_data`、`fd_frame_flags_and_long_data`（`##1` 类 dlc>8、is_fd=true）、`error_frame_and_garbage_are_skipped_counted`（stats.skipped 精确、后续行不受影响）、`extended_id_8_hex`——Review Focus 3。
- [ ] **Step 2:** Run `cargo test -p lumelog-can --test candump`，Expected: FAIL。
- [ ] **Step 3:** 实现 `FrameSource` trait + `CandumpSource`（逐行 split_whitespace，`iface id#data` 或 `(ts) iface id#data`；`#`/`##` 分流；每帧填 `data[..dlc]` 其余补 0）。
- [ ] **Step 4:** Run `cargo test -p lumelog-can --test candump`，Expected: PASS。
- [ ] **Step 5:** CHANGELOG 条目 → commit `feat:can-candump-source` + push。

### Task 5: Vector ASC 源

**Files:**
- Create: `crates/lumelog-can/src/source/asc.rs`
- Create: `crates/lumelog-can/tests/data/asc_basic.asc`（含 `date` 头、`base can 1 ...` 数据行、扩展帧 `x` 后缀、`Rx r` remote、`boe`/`bcs`/`error` 跳过行）
- Test: `crates/lumelog-can/tests/asc.rs`

**Interfaces:**
- Consumes: Task 4 `FrameSource/Line`、Task 1 `Frame/FrameFlags`。
- Produces: `pub struct AscSource<I: Iterator<Item = Line>> { ... }`；`pub fn new(lines: I) -> Self`；`impl FrameSource`。解析 `date` 行取时间基准；数据帧行 `<base> <chan> <ID>x? <Rx|Tx> <d|data> <dlc> <b0> <b1> ...`；扩展帧 id 带 `x`→is_extended；`Rx r`(remote)→is_remote；`boe`/`bcs`/`error`/非帧行 → 跳过计数（spec §7.3.2）。

- [ ] **Step 1:** 写失败测试 `asc.rs`：`parses_data_frames_with_channel_and_id`、`extended_id_x_suffix`、`remote_frame_rx_r`、`skips_boe_bcs_error_counted`（stats.skipped 精确）、`malformed_line_does_not_stop_stream`——Review Focus 3。
- [ ] **Step 2:** Run `cargo test -p lumelog-can --test asc`，Expected: FAIL。
- [ ] **Step 3:** 实现 `AscSource`（以「行含 ` Rx `/` Tx ` 且首 token 可 parse 为 f64 base」判定帧行，其余交跳过分支）。ASC 变体多，只保证 spec 描述形态 + 语料夹具；未覆盖变体进 CHANGELOG 已知限制。
- [ ] **Step 4:** Run `cargo test -p lumelog-can --test asc`，Expected: PASS。
- [ ] **Step 5:** CHANGELOG 条目 → commit `feat:can-asc-source` + push。

### Task 6: 通用正则源

**Files:**
- Create: `crates/lumelog-can/src/source/regex_source.rs`
- Test: `crates/lumelog-can/tests/regex_source.rs`

**Interfaces:**
- Consumes: Task 4 `FrameSource/Line`、`regex` crate、Task 1 `Frame`。
- Produces: `pub struct RegexSource<I: Iterator<Item = Line>> { ... }`；`pub fn new(lines: I, pattern: &str) -> Result<Self, CanError>`（编译期校验命名组含 `id`、`data`，可选 `ts`；缺 `id`/`data` → `CanError::BadRegex`）；`impl FrameSource`。`id` 按 hex 解析（容忍 `0x` 前缀），`data` 按 hex（容忍空格/冒号分隔）→ dlc 取实际字节数，`ts` 按 f64（缺组→None）。

- [ ] **Step 1:** 写失败测试 `regex_source.rs`：`builds_from_pattern_with_named_groups`（含 id/data/ts）、`missing_required_group_errors`（缺 `data`→`BadRegex`，Review Focus 5）、`decodes_custom_log_lines`（对一条自研格式日志正解出 id/data/ts）、`non_matching_line_skipped_counted`、`data_length_drives_dlc`（hex 长度÷2 = dlc）。
- [ ] **Step 2:** Run `cargo test -p lumelog-can --test regex_source`，Expected: FAIL。
- [ ] **Step 3:** 实现 `RegexSource`（`Regex::new` → `captures`；`name("id")`/`name("data")`/`name("ts")`）。
- [ ] **Step 4:** Run `cargo test -p lumelog-can --test regex_source`，Expected: PASS。
- [ ] **Step 5:** CHANGELOG 条目 → commit `feat:can-regex-source` + push。

### Task 7: lumelog-can 解码 CLI（M2 引擎验收）

**Files:**
- Create: `crates/lumelog-can/src/bin/lumelog-can.rs`
- Modify: `crates/lumelog-can/Cargo.toml`（`[[bin]] name="lumelog-can" path="src/bin/lumelog-can.rs"`）
- Create: `crates/lumelog-can/tests/cli_can.rs`
- Create: `crates/lumelog-can/src/tests_common.rs`（`pub mod tests_common`，`#[cfg(test)]`+ 供 CLI/基准：`gen_candump(n, seed) -> Vec<(u64,String)>` 确定性合成）

**Interfaces:**
- Consumes: Task 2 `parse/parse_lenient`、Task 3 `decode_signals`、Task 4–6 三源 + `FrameSource`。
- Produces: CLI 子命令（手工 `std::env::args` 解析，不引 clap）：
  - `lumelog-can dbc --file PATH [--lenient]` → 打印 `{ messages, signals, errors }` JSON（`errors` = lenient 模式跳过的坏行数）。
  - `lumelog-can decode --log PATH --format candump|asc|regex [--regex STR] [--dbc PATH] [--limit N] [--out jsonl|csv]` → 无 `--dbc` 只出 `Frame` 概要（line/id/dlc/data-hex/ts）；带 `--dbc` 对每帧 `decode_signals` 附信号物理值。默认打印前 `--limit`（缺省 20）帧 + 末行 `{ decoded, skipped }`；`--out jsonl|csv` 全量落 stdout。
  - 退出码：无参数/用法错=2；文件不存在=1。
- [ ] **Step 1:** 建 `tests_common::gen_candump`（固定种子，产 `n` 条合法 candump 行，id/data 可预测）。
- [ ] **Step 2:** 写集成测试 `cli_can.rs`（`std::process::Command` 跑 `cargo run -p lumelog-can --bin lumelog-can -- ...`）：`dbc_summary_counts_messages`（对 sample.dbc）、`decode_candump_counts_and_sample`（对 candump_basic.log 断言 decoded/skipped）、`decode_with_dbc_emits_signal_physical`（一条帧解码出的物理值与 Task 3 golden 一致）、`no_args_exits_2`、`missing_file_exits_1`。
- [ ] **Step 3:** 实现 bin（子命令分派 + 选源 → 逐帧 `decode_signals` + 手工 JSONL/CSV 序列化）。
- [ ] **Step 4:** Run `cargo test -p lumelog-can --test cli_can`，Expected: PASS。
- [ ] **Step 5: M2 引擎正确性面**：新增 `#[test] fn decodes_100k_frames_faithfully()`（`tests_common::gen_candump(100_000, seed)` → `CandumpSource` 全解 → 逐帧 id/dlc/data 与生成端记录一致，decoded==100_000，skipped==0）——对齐 §12「10 万帧…解码正确」的引擎侧。Run `cargo test -p lumelog-can --test cli_can decodes_100k`，Expected: PASS。
- [ ] **Step 6:** 吞吐基准（可选 criterion，若不引则用计时测试打印）：10 万帧解码耗时打印进 CHANGELOG（引擎侧参考，UI 流畅 P4 属 Plan 3）。
- [ ] **Step 7:** Run 全量 `cargo test -p lumelog-can` + `cargo clippy -p lumelog-can --all-targets -- -D warnings`，Expected: 全绿/净。CHANGELOG M2 引擎汇总条目 → commit `feat:can-decode-cli` + push。

## Plan-Complete Notes

- **不在本计划**（明确交 Plan 3）：`dbc_import`/`can_parse`/`export_lines`/`export_csv`/`export_jsonl` 的 Tauri IPC 命令层与 channel 流；CanPanel（TanStack Table 报文表格、虚拟滚动、ID/信号/时间过滤、编辑器双向联动、100 万帧流畅）；`Frame`→serde 序列化对齐 §9 批量帧（≤2ms/4096 条聚合）；把 `DocFile` 行流接到 `FrameSource`。
- 对拍夹具 `golden_bitorder.json` 一旦生成并提交，CI 只跑 Rust 测试，不再需要 Python/cantools。
- 位序算法（Task 3）是全计划最高风险点：Motorola `@0` 递推务必以 golden 为唯一判据，不匹配则改实现不改夹具。
