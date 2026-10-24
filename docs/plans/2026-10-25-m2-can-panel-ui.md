# LumeLog Plan 3 (M2 CanPanel + IPC) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or native executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax to track.

**Goal:** 把 Plan 2 交付的纯 Rust `lumelog-can` 引擎接进 Tauri 壳与前端，交付 M2 的另一半：`dbc_import` / `can_parse` IPC 命令，以及 CanPanel 报文表格（列 + 虚拟滚动 + 视图级过滤 + 与编辑器双向联动、行号对齐）。

**Architecture:** 后端在 `app/src-tauri` 命令层新增 CAN 子模块——命令仍是薄转发，重逻辑全在 `lumelog-can`；`dbc_import` 走 `parse_lenient` 得到 `Database` + 错误列表并缓存到 `dbc_id`，`can_parse` 把 `DocFile` 按行喂给 `FrameSource`、批量流式发 `Frame`（`Channel` + 协作式取消，模板为 `search_run`）。前端在 `ipc.ts` 加 CAN 桥接与错误码，新增 `canpanel.ts`（`@tanstack/table-core` 列模型 + 复用 `listview.ts` 的行窗口虚拟化）与纯过滤引擎 `canfilter.ts`，在 `main.ts` 挂一个可折叠的报文 dock，双向联动靠给 `ListView`/编辑器句柄加 `revealLine(line)` 实现。

**Tech Stack:** 后端 Rust 2021（≥1.79）：新增 crate 依赖 `lumelog-can`（本身纯 Rust），DTO 用已有的 `serde`/`serde_json`。前端 Vite + TypeScript 原生 DOM：新增 `@tanstack/table-core`（框架无关）；CodeMirror 6 已在。测试：后端 `cargo test -p lumelog-app`（纯函数 + tempfile 夹具），前端 `vitest`（纯过滤/批量逻辑），渲染与联动为手动清单。

**Spec:** `docs/specs/2026-10-24-lumelog-v1-design.md`（§7.4 CanPanel、§9 IPC 契约中的 `dbc_import`/`can_parse`、§12 M2 验收「CanPanel 联动、行号对齐」、§3 数据按块流动、§8 前端选型）。本计划覆盖 §7.4 + §9 两条 CAN 命令；§7.5 的 `export_lines`/`export_csv`/`export_jsonl` 与行过滤/时序筛选按 §12 属 **M3**，不在本计划（`lumelog-can` 已具 CSV/JSONL 纯函数，M3 直接复用）。

## Global Constraints

- 语言：所有用户可见文案为简体中文（沿用 `ipc.ts` 错误码中文表）。
- 前端无框架（A2）：只用原生 DOM + CodeMirror 6 + `@tanstack/table-core`，不引 React/Vue/Lit。
- DTO 对齐：后端 serde 结构一律 `#[serde(rename_all = "camelCase")]`，前端 `ipc.ts` interface 字段名逐一对应（camelCase）；新增错误码必须同时进 `IpcErrorCode` 联合类型与 `IPC_ERROR_ZH` 表，`ipc.test.ts` 钉住全覆盖。
- IPC 错误码：`CanError` → `IpcError` 映射沿用 `ipc_code()` 稳定字符串——`io` / `dbc_syntax` / `bad_regex` / `regex_compile` / `source_format`；错误形状 `{ code, message, detail? }`（spec §9）。
- 数据按块流动（spec §3）：`can_parse` 永不把整文件一次性交给前端；`Frame` 按 `PARSE_BATCH` 条聚合发帧，长任务带 `request_id` 可取消。
- 绝不 panic：后端每命令逻辑遇畸形帧/坏 DBC/无匹配 message 只跳过或降级（`Frame` 无信号则 `signals: []`、`dataHex` 原样），前端渲染对空 data（remote/error 帧）与 `dlc..=64` 不得越界。
- 行号语义：`lumelog-can` 的 `Frame.line_number` 是 **1-based** 物理行号（Plan 2 `BoxedLines` 契约）；`revealLine` 换算到 CodeMirror（1-based line）与 `ListView`（0-based index）时 MUST 减/加 1 对齐，联动跳行的正确性即 M2 验收「行号对齐」。
- 复用现有模式：`can_parse` 抄 `search_run` 的 Channel/批量/取消骨架；取消复用 `AppDocs::begin_search/cancel/end_search`（本质是通用 `AtomicBool` 注册表）；CanPanel 虚拟滚动抄 `listview.ts` 的行窗口，不引第二套虚拟化库。
- Git：每次 commit 后直接 `git push origin <当前分支>`；commit 前先追加根目录 `CHANGELOG.md`（写清改了哪些文件 / 原因 / 如何验证）；提交消息无空格连写（shell 会剥引号），如 `feat:can-ipc-dbc-import`。
- 阈值：编辑/只读两档 200MB（`DocInfo.mode` 已给），CanPanel 对两种模式的文档都能解析（都经 `DocFile::chunk_at` 取行），不受 mode 限制。

## Review Focus

spec 隐含、但单任务测试不会天然覆盖、最容易伤到使用者的输入类，各由一个任务落钉：

1. **can_parse 流式与取消**：几十万帧时逐帧发 IPC 会风暴，且必须能中途取消而不泄漏半态注册——批量聚合 + 协作式 cancel 钉死（Task 3）。
2. **行号对齐（联动正确性核心）**：点表格行跳编辑器/列表必须落在同一物理源行，含 CRLF 与编辑/只读两模式；`line_number` 的 1-based↔0-based 换算错位是 M2 验收失败点（Task 6/7）。
3. **FD / remote / error 帧进表**：`dlc` 可到 64、remote/error 帧无数据（`dataHex` 空、`signals` 空）不得撑坏列宽或越界（Task 1 DTO + Task 6 渲染）。
4. **过滤是视图级的**：ID 范围 / 消息名 / 信号条件 / 时间窗口过滤 MUST 不改源文档行索引、不改 frame store 本体；空结果、非法区间（起 > 止）、无时间戳帧遇时间窗口时的取舍要明确（Task 5）。
5. **DBC 部分导入可用**：`parse_lenient` 带错误仍产出可用 `Database`；帧的 `can_id` 无匹配 message 时只显示原始 hex、不报错不崩（Task 2 + Task 3 解码分支）。

---

### Task 1: 接入 lumelog-can 依赖 + CAN DTO + from_can 错误映射

**Files:**
- Modify: `app/src-tauri/Cargo.toml`（`[dependencies]` 加 `lumelog-can = { path = "../../crates/lumelog-can" }`）
- Modify: `app/src-tauri/src/dto.rs`（新增 CAN DTO + `From`/转换函数 + `IpcError::from_can`）

**Interfaces:**
- Consumes: `lumelog_can::frame::{Frame, SignalValue}`、`lumelog_can::dbc::{Database, Message}`、`lumelog_can::decode::decode_signals`、`lumelog_can::error::CanError`
- Produces（serde camelCase，全部 `#[derive(Serialize, Clone)]`；`Deserialize` 仅测试往返需要时加）:
  - `pub struct SignalValueJson { pub name: String, pub raw: i64, pub physical: f64, pub enum_text: Option<String>, pub unit: String }`（`enum_text` 用 `skip_serializing_if = "Option::is_none"`；`unit` 直接取 `SignalValue.unit: String`，可为空串）
  - `pub struct FrameJson { pub line_number: u64, pub timestamp_ms: Option<f64>, pub channel: u8, pub can_id: u32, pub can_id_hex: String, pub dlc: u8, pub data_hex: String, pub is_remote: bool, pub is_fd: bool, pub error_frame: bool, pub msg_name: Option<String>, pub signals: Vec<SignalValueJson> }`（注：`Frame`/`FrameFlags` **无 `is_extended`**，故 `FrameJson` 不带该字段）
  - `pub struct DbcMsgSummary { pub id: u32, pub name: String, pub dlc: u8, pub signal_count: u32 }`
  - `pub struct DbcErrorJson { pub line: u64, pub context: String, pub kind: String }`
  - `pub struct DbcInfo { pub dbc_id: u64, pub messages: Vec<DbcMsgSummary>, pub errors: Vec<DbcErrorJson> }`
  - `pub struct CanStatsJson { pub decoded: u64, pub skipped: u64 }`
  - `pub fn frame_json(f: &Frame, db: Option<&Database>) -> FrameJson`：`can_id_hex = format!("0x{:X}", f.can_id)`；`timestamp_ms = f.timestamp.map(|s| s * 1000.0)`（core `timestamp` 为**秒**，DTO 命名 `timestampMs` 为毫秒）；`data_hex` = `f.data[..f.dlc as usize]` 逐字节大写十六进制拼接（remote/error 帧 dlc 可能为 0 → 空串）；`is_remote=f.flags.is_remote`、`is_fd=f.flags.is_fd`、`error_frame=f.flags.is_error_frame`；DBC 匹配仅按 `db.messages.iter().find(|m| m.id == f.can_id)`（`Frame` 无扩展位，不按 extended 区分），命中则 `msg_name = Some(m.name)` 且 `signals = decode_signals(m, &f.data[..f.dlc as usize])` 逐项映射，否则 `msg_name = None`、`signals = []`。
  - `impl IpcError { pub fn from_can(e: CanError) -> Self }`：`Io→("io",None)`；`DbcSyntax{line,context,kind}→("dbc_syntax", Some(format!("第 {line} 行：{context}（{kind}）")))`；`BadRegex(m)→("bad_regex", Some(m))`；`RegexCompile(m)→("regex_compile", Some(m))`；`SourceFormat(m)→("source_format", Some(m))`。

- [ ] **Step 1: 加依赖 + 扩展 dto.rs**

在 `Cargo.toml` 加 `lumelog-can` 路径依赖。在 `dto.rs` 追加上述结构（沿用文件顶部 `use serde::{Deserialize, Serialize};`，CAN 结构加 `use lumelog_can::...`）。`frame_json` 里 message 匹配的 extended 判断写成 `let ext_ok = (!m.is_extended) == (!f.is_extended);`——扩展帧与标准帧 `can_id` 数值可能相同，须区分。**精确匹配以 `!m.is_extended != !f.is_extended` 为准**（若 Plan 2 帧未携带 extended 位、`is_extended` 恒 false，则退化为仅按 `id` 匹配，不阻断）。

- [ ] **Step 2: 写 DTO 转换与错误映射的单元测试**

在 `dto.rs` 末尾 `#[cfg(test)] mod can_tests`：
1. `frame_json_maps_fields`：手工构造一个 `Frame`（candump 风格 `can_id=0x1A0`、`dlc=8`、data 已知、`timestamp_ms=Some(1.5)`），`frame_json(&f, None)` → 断言 `can_id_hex == "0x1A0"`、`data_hex` 等于期望大写串、`signals.is_empty()`、`msg_name.is_none()`。
2. `frame_json_decodes_signals_when_matched`：用 `lumelog_can::dbc::parse(include_str!(...sample.dbc路径见下))` 得 `Database`，对 `EngineData`（id=0x100）构造 data=`[0x00,0x00,0x96,0x00,...]`（Speed raw=150 → physical 150），`frame_json(&f, Some(&db))` → `msg_name == Some("EngineData")`、signals 含 name=="Speed"。sample.dbc 在 `crates/lumelog-can/tests/data/sample.dbc`——`include_str!` 用相对路径 `concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/lumelog-can/tests/data/sample.dbc")`。
3. `error_frame_has_no_signals_and_empty_data`：error 帧（`dlc=0`、`error_frame=true`）→ `data_hex.is_empty()`、`signals.is_empty()`、不 panic。
4. `from_can_codes_are_stable`：五个 `CanError` 变体各断言 `.code` 字符串，钉 §9 契约。
5. `frame_json_serializes_camel_case`：`serde_json::to_value(frame_json(&f, None))` 断言键为 `lineNumber`/`canIdHex`/`timestampMs`/`isRemote`/…（camelCase）。

Run: `cargo test -p lumelog-app can_tests`
Expected: 先 FAIL（类型/函数未定义）→ 实现后 PASS

- [ ] **Step 3: 全量校验 + CHANGELOG + commit**

Run: `cargo clippy -p lumelog-app --all-targets -D warnings`（净）
```bash
git add app/src-tauri/Cargo.toml app/src-tauri/src/dto.rs CHANGELOG.md
git commit -m feat:can-ipc-dto-and-error-map && git push origin main
```

---

### Task 2: DbcStore 状态 + `dbc_import` 命令

**Files:**
- Modify: `app/src-tauri/src/state.rs`（加 `dbc` 缓存）
- Modify: `app/src-tauri/src/commands.rs`（`dbc_import` + 纯函数 `import_dbc`）
- Modify: `app/src-tauri/src/lib.rs`（`generate_handler!` 注册 `commands::dbc_import`）

**Interfaces:**
- Consumes: `lumelog_can::dbc::{parse_lenient, CanParseReport, Database}`、`Task 1 DbcInfo/DbcMsgSummary/DbcErrorJson/IpcError::from_can`
- Produces:
  - state：`pub fn insert_dbc(&self, db: Arc<Database>) -> u64`、`pub fn get_dbc(&self, id: u64) -> Option<Arc<Database>>`（`AppDocs` 新增字段 `dbc: Mutex<HashMap<u64, Arc<Database>>>` + `next_dbc_id: AtomicU64`）
  - commands：`pub fn import_dbc(path: &Path) -> Result<(Database, DbcInfo-ish), IpcError>` 的纯内核 `fn dbc_info_of(report: &CanParseReport) -> (Vec<DbcMsgSummary>, Vec<DbcErrorJson>)`
  - `#[tauri::command] pub fn dbc_import(state, path: String) -> Result<DbcInfo, IpcError>`

- [ ] **Step 1: state.rs 加 dbc 缓存**

`AppDocs` 加 `dbc: Mutex<HashMap<u64, Arc<Database>>>` 与 `next_dbc_id: AtomicU64`（`use lumelog_can::dbc::Database;`）。仿 `insert/get` 写 `insert_dbc`/`get_dbc`（克隆 `Arc`）。`#[cfg(test)]` 补一个 `dbc_store_roundtrip`：插两个 db 得不同 id、`get_dbc` 命中、miss 返回 `None`。

- [ ] **Step 2: 写失败测试（import_dbc 对 sample.dbc）**

`commands.rs` 的 `#[cfg(test)]`：`dbc_import_reads_sample_dbc` — `let p = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/lumelog-can/tests/data/sample.dbc"));` 调纯函数 `let info = import_dbc_info(p).unwrap();` 断言 `info.errors.is_empty()`、`info.messages` 含名为 `"EngineData"` 的 summary 且其 `signal_count >= 4`、`info.messages.iter().find(|m| m.name=="SensorMsg").unwrap().signal_count == 2`。

Run: `cargo test -p lumelog-app dbc_import`
Expected: FAIL（`import_dbc_info` 未定义）

- [ ] **Step 3: 实现 import_dbc_info + dbc_import 命令**

`pub fn import_dbc_info(path: &Path) -> Result<CanParseReport, IpcError>`：`std::fs::read(path)` → `String::from_utf8_lossy`（DBC 为 ASCII/Latin，坏字节不致命）→ `parse_lenient(&s).map_err(IpcError::from_can)?`。命令 `dbc_import(state, path)`：`import_dbc_info(Path::new(&path))?`；由 `report.database` 生成 `Vec<DbcMsgSummary>`（`id=if m.is_extended { m.id | 0x8000_0000 } else { m.id }`、`signal_count=m.signals.len() as u32`）、由 `report.errors` 生成 `Vec<DbcErrorJson>`；`let dbc_id = state.insert_dbc(Arc::new(report.database));`；返回 `DbcInfo{ dbc_id, messages, errors }`。文件不存在/读失败 → `IpcError::new("io", ...)`。

- [ ] **Step 4: 注册 handler + 全绿**

`lib.rs` 的 `generate_handler![...]` 追加 `commands::dbc_import`。
Run: `cargo test -p lumelog-app`（含新测试全绿）+ `cargo clippy -p lumelog-app --all-targets -D warnings`（净）+ `cargo check -p lumelog-app`

- [ ] **Step 5: CHANGELOG + commit**
```bash
git add app/src-tauri/src/state.rs app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs CHANGELOG.md
git commit -m feat:can-ipc-dbc-import && git push origin main
```

---

### Task 3: `can_parse` 流式命令 + DocFile→行流 + can_cancel + can_decode_snippet

**Files:**
- Modify: `app/src-tauri/src/commands.rs`（`can_parse`、`can_cancel`、`can_decode_snippet`、纯内核 `parse_frames`、`doc_lines`）
- Modify: `app/src-tauri/src/lib.rs`（注册三命令）

**Interfaces:**
- Consumes: `Task 1 FrameJson/CanStatsJson/frame_json`、`lumelog_can::source::{FrameSource, Line, CandumpSource, AscSource, RegexSource}`（注：无 `BoxedLines`/`SourceKind`，源为泛型 `Source<I: Iterator<Item=Line>>`，`Line=(u64,String)` 行号由调用方提供）、`lumelog_can::frame::Frame`、`lumelog_core::doc::DocFile`、`state::{AppDocs::begin_search/cancel/end_search/get_dbc}`
- Produces:
  - `fn doc_lines(doc: &mut DocFile) -> Result<Vec<Line>, IpcError>`（`Line=(u64,String)`）：分块（每块 `≤ MAX_CHUNK_LINES`）调 `doc.chunk_at`、按 `/\r\n|\n/` 切行（末块去掉尾部空行，对齐 `ipc.ts::loadWindow` 语义），产出 `Vec<(line_index+1 /*1-based*/, text)>`。命令层再 `Box::new(vec.into_iter())` 作 `Box<dyn Iterator<Item=Line>>` 传给具体源。
  - `fn parse_frames(src: &mut dyn FrameSource, db: Option<&Database>, emit: &mut dyn FnMut(Vec<FrameJson>), cancelled: &dyn Fn() -> bool) -> (u64, u64)`：接收已按 `source_kind` 在命令层构造好的源对象（`RegexSource` 需要 pattern，故选源/构造源留在命令层，`parse_frames` 只对抽象 `FrameSource` 逐帧解码，测试可直接传构造好的源）。逐 `next_frame`，`cancelled()` 为真即中断；每帧 `frame_json(&f, db)` 入 batch，满 `PARSE_BATCH=512` 就 `emit(take(batch))`；结束 flush 余量；最后 `let s = src.stats(); (s.decoded, s.skipped)`。
  - `#[tauri::command] pub fn can_parse(state, doc_id, dbc_id: Option<u64>, source_kind: String, regex: Option<String>, on_batch: Channel<Vec<FrameJson>>, on_done: Channel<CanStatsJson>) -> Result<u64, IpcError>`
  - `#[tauri::command] pub fn can_cancel(state, request_id: u64) -> Result<(), IpcError>`
  - `#[tauri::command] pub fn can_decode_snippet(state, text: String, dbc_id: Option<u64>, source_kind: Option<String>) -> Result<FrameJson, IpcError>`（编辑器选中「识别为报文」）

- [ ] **Step 1: 写失败测试（parse_frames 忠实解码 tempfile candump）**

`commands.rs` `#[cfg(test)]`：
1. `parse_frames_decodes_candump_with_signals`：用 `tempfile::NamedTempFile` 写若干 candump 行（含 ` (1.5) can0 100#0000960012340000` 与一行畸形），`DocFile::open` 后 `doc_lines` → `SourceKind::Candump` 建 `CandumpSource`，`parse_frames(src, None, &mut collect, &||false)` → 断言 `decoded >= 1`、`skipped == 1`、收到的 `FrameJson` 里 `can_id==0x100`。
2. `parse_frames_stops_on_cancel`：`cancelled` 回调第 2 帧后返回 `true`，断言 emit 的总帧数 < 全部、且函数正常返回不 panic。
3. `doc_lines_preserves_line_order`：对一个已知多行文件（含 CRLF），`doc_lines` 收集的行数 == `doc.line_count()`，逐行文本与源一致（末行处理）。

Run: `cargo test -p lumelog-app parse_frames`
Expected: FAIL（未定义）

- [ ] **Step 2: 实现 doc_lines + parse_frames**

`doc_lines`：`let mut out = Vec::new(); let total = doc.line_count(); let mut start = 0; while start < total { let count = (MAX_CHUNK_LINES as usize).min(total - start); let ch = doc.chunk_at(start, count).map_err(IpcError::from_core)?; // ch.text 含行尾，去掉整体末尾一个 \r\n 或 \n 后 split(/\r\n|\n/)，第 i 行→ `(ch.start_line + i) as u64 + 1`; start += count }`；返回 `Vec<Line>`。`PARSE_BATCH=512` 常量。`parse_frames(src: &mut dyn FrameSource, db, emit, cancelled)`：`while let Some(f) = src.next_frame() { if cancelled() { break } batch.push(frame_json(&f, db)); if batch.len()>=PARSE_BATCH { emit(std::mem::take(&mut batch)) } }` 结尾 `if !batch.is_empty(){emit(batch)}`；最后 `let s = src.stats(); (s.decoded, s.skipped)`。

- [ ] **Step 3: 实现 can_parse / can_cancel / can_decode_snippet 命令**

`can_parse`：`let (req_id, cancel) = state.begin_search();`；取 doc（miss → `end_search` + `unknown_doc`）；`doc_lines(&mut doc)` 取 `Vec<Line>`；按 `source_kind` 字符串匹配（`"candump"|"asc"|"regex"|other→source_format`）选源并 `Box<dyn FrameSource>`（`regex` 且 `regex=None` → `bad_regex`；`RegexSource::new(lines.into_iter(), pat).map_err(from_can)?`）；`db = dbc_id.and_then(|i| state.get_dbc(i))`；跑 `parse_frames`，`emit` 里 `on_batch.send(...)`（忽略 send 错误=前端已断开），`cancelled` 里读 `cancel` 标志；`state.end_search(req_id)`；`on_done.send(CanStatsJson{decoded,skipped})`；`Ok(req_id)`。`can_cancel` 调 `state.cancel(request_id)`（miss → `unknown_request`）。`can_decode_snippet`：`kind = source_kind.unwrap_or("candump")`；把 `text` 作单行 `(1, text)` 喂对应源（仅 candump/asc），取第一个 `Frame`，无 → `IpcError::new("source_format",...)`；有 → `frame_json(&f, dbc_id.and_then(get_dbc).as_deref())`。

- [ ] **Step 4: 注册 handler + 全绿**

`lib.rs` 追加 `commands::can_parse, commands::can_cancel, commands::can_decode_snippet`。
Run: `cargo test -p lumelog-app` + `cargo clippy -p lumelog-app --all-targets -D warnings` + `cargo check -p lumelog-app`

- [ ] **Step 5: CHANGELOG + commit**
```bash
git add app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs CHANGELOG.md
git commit -m feat:can-ipc-parse-stream && git push origin main
```

---

### Task 4: 前端 ipc.ts CAN 桥接 + 新错误码 + vitest

**Files:**
- Modify: `app/frontend/src/ipc.ts`（CAN interface + 包装函数 + 4 错误码入表）
- Modify: `app/frontend/src/ipc.test.ts`（错误码覆盖 + 桥接纯逻辑）

**Interfaces:**
- Consumes: Task 1–3 的 `DbcInfo`/`FrameJson`/`CanStatsJson`（camelCase）、`dbc_import`/`can_parse`/`can_cancel`/`can_decode_snippet`
- Produces:
  - interface `SignalValueJson { name; raw:number; physical:number; enumText?:string; unit?:string }`
  - interface `FrameJson { lineNumber:number; timestampMs?:number; channel:number; canId:number; canIdHex:string; dlc:number; dataHex:string; isRemote:boolean; isFd:boolean; isExtended:boolean; errorFrame:boolean; msgName?:string; signals:SignalValueJson[] }`
  - interface `DbcMsgSummary { id:number; name:string; dlc:number; signalCount:number }` / `DbcErrorJson { line:number; context:string; kind:string }` / `DbcInfo { dbcId:number; messages:DbcMsgSummary[]; errors:DbcErrorJson[] }` / `CanStatsJson { decoded:number; skipped:number }`
  - `dbcImport(path:string): Promise<DbcInfo>`；`canParse(args:{docId:number; dbcId?:number|null; sourceKind:string; regex?:string|null; onBatch:(f:FrameJson[])=>void; onDone:(s:CanStatsJson)=>void}): Promise<number>`（抄 `searchRun` 的双 Channel 形状）；`canCancel(requestId:number): Promise<void>`；`canDecodeSnippet(args:{text:string; dbcId?:number|null; sourceKind?:string|null}): Promise<FrameJson>`
  - 错误码 `IpcErrorCode` 加 `"dbc_syntax"|"bad_regex"|"regex_compile"|"source_format"`，`IPC_ERROR_ZH` 加中文：`dbc_syntax:"DBC 解析出错%s"`、`bad_regex:"解析正则无效（需含命名组 id/data）%s"`、`regex_compile:"正则编译失败%s"`、`source_format:"日志源格式无法识别%s"`。

- [ ] **Step 1: 写失败测试**

`ipc.test.ts`：`ipc_error_code_maps_to_zh_text` 已遍历 `IPC_ERROR_CODES`——新增码进表后该用例自动要求四条都有中文（先 FAIL）；再加 `can_parse_wrappers_exist`：断言 `typeof dbcImport/canParse/canCancel/canDecodeSnippet === "function"`。

Run: `npm --prefix app/frontend test -- ipc`
Expected: FAIL（码/函数未定义）→ 实现后 PASS

- [ ] **Step 2: 实现桥接**

在 `ipc.ts` 加 interface（照抄现有 `MatchJson` 风格 + `IpcErrorPayload` 无关），加四个包装函数（`canParse` 用 `new Channel<FrameJson[]>()`/`Channel<CanStatsJson>()`），补错误码联合与中文表。

- [ ] **Step 3: 全绿 + commit**

Run: `npm --prefix app/frontend test`（全绿）+ `npm --prefix app/frontend run build`（`tsc --noEmit` 通过）
```bash
git add app/frontend/src/ipc.ts app/frontend/src/ipc.test.ts CHANGELOG.md
git commit -m feat:can-ipc-frontend-bridge && git push origin main
```

---

### Task 5: CanPanel 视图过滤引擎（纯函数）+ vitest

**Files:**
- Create: `app/frontend/src/canfilter.ts`
- Test: `app/frontend/src/canfilter.test.ts`

**Interfaces:**
- Consumes: `ipc.ts` `FrameJson`
- Produces:
  - `export interface CanFilter { idFrom?:number; idTo?:number; msgName?:string; signalName?:string; signalOp?:">"|"<"|">="|"<="|"=="; signalValue?:number; timeFromMs?:number; timeToMs?:number }`
  - `export function applyCanFilter(frames: FrameJson[], f: CanFilter): FrameJson[]`（纯，不改入参数组，不改 frame 对象）

- [ ] **Step 1: 写失败测试**

`canfilter.test.ts`（用几帧固定 `FrameJson`）：
1. `no_filter_returns_all`：空 `CanFilter` → 原样返回（长度相等、顺序不变）。
2. `id_range_inclusive`：`{idFrom:0x100, idTo:0x1ff}` 只留 `canId` 落在闭区间；**起 > 止时返回空**（非法区间不抛错）。
3. `msg_name_matches`：`{msgName:"EngineData"}` 只留 `msgName==="EngineData"`（`undefined` 的被滤掉）。
4. `signal_condition`：`{signalName:"Speed", signalOp:">", signalValue:60}` 只留有名为 Speed 且 `physical>60` 的帧；`==` 对浮点用 `Object.is` 精确比较即可（钉死语义）。
5. `time_window_drops_ts_none`：`{timeFromMs:1000,timeToMs:2000}` 只留 `timestampMs` 落在闭区间；`timestampMs` 为 `undefined` 的帧在此条件下被排除。
6. `does_not_mutate`：调用后原数组与元素 `signals` 未被修改（`structuredClone` 前后比对）。

Run: `npm --prefix app/frontend test -- canfilter`
Expected: FAIL（模块未定义）

- [ ] **Step 2: 实现 applyCanFilter**

逐帧谓词与（AND）：ID 区间 `f.idFrom!==undefined && (c<f.idFrom||c>f.idTo ?? c<f.idFrom)`——明确：仅给 `idFrom` 时下界、仅给 `idTo` 时上界、都给时闭区间且 `idFrom>idTo` 直接判不匹配。信号条件遍历 `signals` 找 `name===signalName` 取 `physical` 比较。返回 `frames.filter(...)`（filter 天然不改入参）。

- [ ] **Step 3: 全绿 + commit**

Run: `npm --prefix app/frontend test`
```bash
git add app/frontend/src/canfilter.ts app/frontend/src/canfilter.test.ts CHANGELOG.md
git commit -m feat:can-panel-filter-engine && git push origin main
```

---

### Task 6: CanPanel 组件（table-core 列模型 + 行窗口虚拟滚动 + 联动回调）

**Files:**
- Create: `app/frontend/src/canpanel.ts`
- Test: `app/frontend/src/canpanel.test.ts`（store/批量拼接的纯逻辑）
- Modify: `app/frontend/src/listview.ts`（加 `revealLine(index0:number)`）
- Modify: `app/frontend/src/editor.ts`（EditorHandle 加 `revealLine(line1:number)`）
- Modify: `app/frontend/package.json`（`@tanstack/table-core` 依赖）+ 安装

**Interfaces:**
- Consumes: `ipc.ts` `FrameJson`/`canParse`、`Task 5 applyCanFilter/CanFilter`、`listview.ts` 的 `makeVirtual` 行窗口思路
- Produces:
  - `export class FrameStore { push(frames: FrameJson[]): void; all(): readonly FrameJson[]; clear(): void; get length(): number }`（累积批量、保留插入顺序）
  - `export class CanPanel { constructor(host: HTMLElement, opts: { onRowActivate(lineNumber:number):void; onError(e:string):void }); attachStore(store: FrameStore): void; setFilter(f: CanFilter): void; render(): void; destroy(): void }`
  - `listview.ts`: `ListView.prototype.revealLine(index0: number)`（滚到 0-based 行）；`editor.ts`: `revealLine(line1: number)`（CodeMirror `EditorView.lineWrapping`… 用 `dispatch({selection:{anchor: lineStart(line1)}, ...})` + `scrollIntoView`）

- [ ] **Step 1: 装依赖 + 写 FrameStore 失败测试**

`npm --prefix app/frontend install @tanstack/table-core@^8`。`canpanel.test.ts`：`frame_store_appends_in_order`（两次 `push` 后 `all()` 顺序为插入序、`length` 累加）、`frame_store_clear`（`clear` 后 `length===0`）。先 FAIL（类未定义）。

- [ ] **Step 2: 实现 FrameStore + CanPanel**

`FrameStore` 内部 `FrameJson[]`。`CanPanel` 用 `@tanstack/table-core` `createTable({ data: filtered, columns, getCoreRowModel })` 拿列/行模型，**DOM 渲染手动**：表头取 `table.getFlatHeaders()`；表体只渲染可见行窗口（仿 `listview.ts` 的 ±缓冲：`top = max(0, first-buffer)`、`bottom = min(n, first+viewport+buffer)`，上下 spacer 撑高），行数据 = `applyCanFilter(store.all(), currentFilter)`。列（§7.4）：行号、时间戳（相对/绝对 + ms/µs 切换按钮，`timestampMs` 空显示 `–`）、通道、ID（hex/dec 切换）、DLC、信号（点击行展开子行显示 `signals`：name / raw / physical / enumText / unit）、原始 hex（`dataHex`，空则 `–`）。remote/error 帧 signals 为空、dataHex 可能空 → 单元格降级显示，不报错。行 `onclick` → `opts.onRowActivate(frame.lineNumber)`（1-based）。渲染纯 DOM（`document.createElement`），无 innerHTML 拼接帧数据（转义安全 + 性能）。

- [ ] **Step 3: listview/editor revealLine**

`listview.ts`：`revealLine(index0)` → 复用现有滚动定位（把该 index 滚进视口、高亮闪烁一次）。`editor.ts`：给 `EditorHandle` 加 `revealLine(line1)`：`const cm = this.view; const ln = cm.state.doc.line(Math.max(1, Math.min(line1, cm.state.doc.lines))); cm.dispatch({ selection:{anchor: ln.from}, effects: EditorView.scrollIntoView(ln.from,{y:"center"}) })`。

Run: `npm --prefix app/frontend test -- canpanel`（全绿）

- [ ] **Step 4: 全绿 + commit**

Run: `npm --prefix app/frontend test` + `npm --prefix app/frontend run build`
```bash
git add app/frontend/src/canpanel.ts app/frontend/src/canpanel.test.ts app/frontend/src/listview.ts app/frontend/src/editor.ts app/frontend/package.json app/frontend/package-lock.json CHANGELOG.md
git commit -m feat:can-panel-table-virtual && git push origin main
```

---

### Task 7: main.ts 集成 CanPanel（报文 dock + 导入/解析 + 双向联动）

**Files:**
- Modify: `app/frontend/src/main.ts`（layout 加 `can-dock` + 工具栏按钮；接线 `dbcImport`/`canParse`/`canDecodeSnippet`/`CanPanel`/`FrameStore`；行点击→`revealLine`）
- Modify: `app/frontend/index.html`（可选：dock 容器样式类已在 canpanel 用，如需 CSS 加 `app/frontend/src/*.css` 或内联）

**Interfaces:**
- Consumes: Task 4 `dbcImport/canParse/canCancel/canDecodeSnippet`、Task 6 `CanPanel/FrameStore` + `listview/editor.revealLine`、Task 5 `applyCanFilter/CanFilter`、现有 `tabs.active()`/`editors`/`lists`

- [ ] **Step 1: 报文 dock + 工具栏按钮**

`layout()` 的 `center` 里，`search-panel` 之后追加 `can-dock`（`display:none` + 一个高度可拖拽容器）和一个 `canpanelHost`。工具栏 `bar.append(...)` 加：`报文面板`（toggle dock 显隐）、`导入 DBC`（`openDialog` 选 `*.dbc` → `dbcImport` → 存 `dbcId`+缓存 `messages`/`errors` 到 `Map<docId-ish>`；有 `errors` 时 toast 提示「已宽容导入，N 处告警」）、`解析报文`（对 `tabs.active()` 文档：弹源选择 candump/asc/regex(regex 输入框) → 新建 `FrameStore` + `CanPanel` → `canParse({docId, dbcId, sourceKind, regex})` 把 `onBatch` 喂 `store.push`+`panel.render()`、`onDone` 更新状态栏计数）；解析中按钮变「停止」→ `canCancel(reqId)`。`refreshPanels()` 里按活动文档切换 panel 显示的 store（每 docId 一份 `FrameStore`，`Map<number, FrameStore>` + `Map<number, CanPanel>`，close doc 时 `destroy` + 删）。

- [ ] **Step 2: 双向联动**

`CanPanel` 的 `opts.onRowActivate(lineNumber)` → `const a=tabs.active(); if(lists.has(a.info.docId)) lists.get(...).revealLine(lineNumber-1); else editors.get(...)?.revealLine(lineNumber)`（列表 0-based 减 1，CodeMirror 1-based 直接用）。编辑器侧「识别为报文」：在编辑器选中处加一个操作（palette 命令或工具栏按钮）：取 `view.state.sliceDoc(sel.from, sel.to)` → `canDecodeSnippet({text, dbcId})` → 把返回帧以 toast/浮层显示（或临时插入面板高亮该行）。源选择与 dbcId 从当前 Tab 关联状态取。

- [ ] **Step 3: 类型/构建全绿 + 手动验收**

Run: `npm --prefix app/frontend run build`（`tsc --noEmit` 通过）+ `npm --prefix app/frontend test`
手动清单（写进 `docs/test-checklist.md` 或 CHANGELOG）：打开 `crates/lumelog-can/tests/data/candump_basic.log` → 导入 `sample.dbc` → 解析（candump）→ 表格出现帧、点行跳源码行、ID 范围过滤生效、Speed 信号展开显示 physical=raw、错误帧行不崩。

- [ ] **Step 4: CHANGELOG + commit**
```bash
git add app/frontend/src/main.ts app/frontend/index.html CHANGELOG.md
git commit -m feat:can-panel-integration && git push origin main
```

---

## 收尾（全计划完成后）

1. 后端：`cargo test -p lumelog-app` + `cargo clippy --workspace --all-targets -D warnings` 净。
2. 前端：`npm --prefix app/frontend test` + `npm --prefix app/frontend run build` 绿。
3. 派 CodeReview 子代理审整条分支（`current_session`，提示：不 flag `crates/lumelog-core/tests/e2e_large_file.rs` 既有 bench 噪声）。
4. 一个修复波 + 定向复审。
5. M2 达成（引擎 Plan 2 + CanPanel Plan 3）：更新 `docs/claude.md` 里程碑状态。导出三件套留 M3。
