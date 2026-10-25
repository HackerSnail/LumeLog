# M3 子项目 A · 增量2：文本日志 时序筛选 + export_lines Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在只读文本日志视图上加"时间戳抽取正则 + 内置格式"时序筛选（无抽取正则时退化为行号区间），并把当前"文本条件 ∧ 时序 ∧ 行号"选中的原始日志行按字节导出到文件（保留原编码/行尾），导出为可取消的流式长任务。

**Architecture:** 扩展 `lumelog_core::filter::LineFilter` 为组合谓词（文本 AND ∧ 时间 ∧ 行号区间，时间用新增 `chrono` 内置格式集）；`filter_run` 复用同一谓词。导出逻辑落 **core**（`line_byte_range`/`read_window` 是 `pub(crate)`）：新增 `lumelog_core::export::run` 逐行取原始字节写 `writer`；`lumelog-app::export_lines` 命令开文件 + `spawn_blocking` + 流式进度/取消（镜像 `filter_run`/`can_parse`）。前端查询面板加"时序"区 + "导出"按钮。

**Tech Stack:** Rust（`lumelog-core` filter/export + `chrono` + `regex`；`lumelog-app` + `tauri::ipc::Channel`）、TypeScript 原生 DOM。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm --prefix app/frontend test` / `npm --prefix app/frontend run build`（tsc）。

**Spec:** `docs/specs/2026-10-25-m3-timeline-export-design.md`。承接增量1 `docs/specs/2026-10-25-m3-filter-highlight-design.md`。

## Global Constraints

- 简体中文 UI；DTO `#[serde(rename_all="camelCase")]`，可缺省字段 `Option` + `skip_serializing_if`。
- `char_map` 不过 IPC（沿用增量1）。不整文件驻留：导出逐行原始字节流式写；`filter_run` 文本/时序判定分块。
- 命令参数 camelCase↔snake_case 由 Tauri 自动转换；异步命令用同步 `fn`+`tauri::async_runtime::spawn_blocking`（`State<'_>` 不入 `'static` 闭包），worker 内经 `AppHandle`+`Manager` 取 `AppDocs` 做 `end_search`。
- 逐任务 TDD（red→green）；每任务四门全绿方可提交；commit 前追加根 `CHANGELOG.md`（时间倒序、写清文件/原因/验证）；commit 消息无空格无特殊字符；commit 后直接 push；分支开发、`--no-ff` 合 `main`。
- 分支基：从 `main`（当前 `c2a7057`）新建 `feat/m3-timeline-export`。

## Review Focus（spec 隐含、无单任务测试易覆盖不到、最易伤使用者）

1. **抽取正则缺 `ts` 命名组**必须同步报错（`invalid_pattern`），不得静默"全部保留"或 panic。
2. **时序与行号是两条独立 AND 轴**：后端 `time`/`line_range` 各自可选、都给则都成立才保留；UI 层"退化"只体现在抽取正则为空时发送 `line_from/line_to` 而非 `time`。`keep_when_no_ts` 仅在有 `time` 时有意义。
3. **原始字节导出逐字节等于源切片**：`line_byte_range` 含行尾；末行无换行时 end=文件大小；首行 BOM 随原始字节保留；CRLF 原样。
4. **取消/写错都始终 `on_done` + `end_search` 恰好一次**：`export::run` 不返回 `Result`，读/写错吸收为 `errored`；worker 末必 `flush`+`on_done.send`+`end_search`。
5. **时间解析边界**：`iso` 兼容 `...Z`/`±HH:MM` 偏移与无偏移（视为 UTC）、可选 `.fff`/`.ffffff`；`epoch_s` 小数秒；解析失败的单行按 `keep_when_no_ts` 处理，不 panic。

---

## Task 1: `lumelog_core::filter` 时序 + 行号扩展（含 `chrono`）

**Files:**
- Modify: `Cargo.toml`（root `[workspace.dependencies]` 加 `chrono = { version = "0.4", default-features = false, features = ["std"] }`）
- Modify: `crates/lumelog-core/Cargo.toml`（`[dependencies]` 加 `chrono = { workspace = true }`）
- Modify: `crates/lumelog-core/src/filter.rs`
- Test: `crates/lumelog-core/src/filter.rs`（扩 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `regex::{Regex,RegexBuilder}`、`chrono::{DateTime, NaiveDateTime}`。
- Produces（在增量1 `CondSpec/CondMode/CondKind/Cu16Range/LineFilter` 基础上）：
  - `pub enum TimeFormat { Iso, EpochSec, EpochMillis }`
  - `pub struct TimeSpec { pub extract: String, pub format: TimeFormat, pub from: Option<String>, pub to: Option<String>, pub keep_when_no_ts: bool }`
  - `LineFilter::build(specs: Vec<CondSpec>, case_sensitive: bool, time: Option<TimeSpec>, line_range: Option<(u64,u64)>) -> Result<LineFilter, CoreError>`（签名新增后两参）
  - `pub fn keep(&self, line_no: u64, text: &str) -> Option<Vec<Cu16Range>>`（签名新增 `line_no`，0-based）
  - 私有 `fn parse_time_ms(s: &str, format: TimeFormat) -> Option<f64>`

- [ ] **Step 1：改现有单测调用 + 写失败测试**。增量1 的 `LineFilter::build(specs, case)` 一律补成 `build(specs, case, None, None)`，`lf.keep("文本")` 一律改为 `lf.keep(0, "文本")`（无时序/无行号时行为不变）；再追加：
  - `line_range_keeps_only_within`：`build(vec![include "x"], true, None, Some((2,3)))` → `keep(0,"x")` None、`keep(1,"x")`(第2行) Some、`keep(2,"x")`(第3行) Some、`keep(3,"x")`(第4行) None。
  - `time_iso_window`：`include` 无 + `time{extract:"(?<ts>\\d{4}-\\d-\\d{2} \\d{2}:\\d{2}:\\d{2})", iso, from:"2026-01-01 00:00:00", to:"2026-01-02 00:00:00"}` → `keep(0,"2026-01-01 06:00:00 ok")` Some、`keep(0,"2025-12-31 23:00:00")` None。
  - `time_epoch_seconds`：`format=EpochSec`，`from=Some("1000")`（=1_000_000ms）、行 `ts=1500`（1_500_000ms）Some、`ts=500` None。
  - `keep_when_no_ts_true_retains_unparsable`：行无 `ts` 组命中，`keep_when_no_ts=true` → Some（空区间），`false` → None。
  - `extract_missing_ts_group_errors`：`build(..., Some(time{extract:"(a)"...}))` → `Err(InvalidPattern)`。
  - `iso_offset_and_fraction`：`2026-01-01T00:00:00.500Z` 与 `2026-01-01T08:00:00+08:00` 解析为相同毫秒。
- [ ] **Step 2：** `cargo test -p lumelog-core filter::` → FAIL（新签名/类型未实现）。
- [ ] **Step 3：实现**：`LineFilter` 加 `time: Option<CompiledTime>` + `line_range: Option<(u64,u64)>` 字段；`CompiledTime{ extract: Regex, format, from_ms: Option<f64>, to_ms: Option<f64>, keep_when_no_ts }`。`build`：编译抽取正则（校验有命名组 `ts`，无则 `InvalidPattern`），`from`/`to` 经 `parse_time_ms` 预解析（失败 `InvalidPattern`）。`keep(line_no,text)` 顺序：`line_range` 含 `line_no+1`？否则 None → 文本 AND（产区间）→ `time`：抽取 `ts` 组 → `parse_time_ms` → None 走 `keep_when_no_ts`（真则保留、假 None）；否则比 `from_ms≤t≤to_ms`（各向可选）。`parse_time_ms`：Iso 先 `DateTime::parse_from_rfc3339→timestamp_millis as f64`，退 `NaiveDateTime::parse_from_str` 两 pattern 当 UTC；EpochSec `f64*1000`；EpochMillis `f64`。
- [ ] **Step 4：** `cargo test -p lumelog-core` PASS（含更新后的增量1 用例）+ `cargo clippy -p lumelog-core --all-targets -- -D warnings` 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:core-filter-time-lineaxis` + push。

---

## Task 2: `lumelog_core::export` 原始字节导出（纯 scan+write）

**Files:**
- Create: `crates/lumelog-core/src/export.rs`
- Modify: `crates/lumelog-core/src/lib.rs`（`pub mod export;`）
- Test: `crates/lumelog-core/src/export.rs`（夹具临时文件，仿 `docfile.rs`/`search.rs` 的 `temp_log`）

**Interfaces:**
- Consumes: `DocFile::{line_count, chunk_at, line_byte_range, read_window}`、`crate::filter::LineFilter`、`crate::error::CoreError`。
- Produces:
  - `pub struct ExportStats { pub written: u64, pub scanned: u64, pub cancelled: bool, pub errored: bool }`
  - `pub fn run<W: std::io::Write>(doc: &mut DocFile, matcher: &LineFilter, writer: &mut W, on_progress: &mut dyn FnMut(u64), cancel: &dyn Fn() -> bool) -> ExportStats`
  - 内部逐行：分块 `chunk_at` 取行文本（去尾换行同 `DocLines`），`matcher.keep(line_no, text)` Some → 取 `line_byte_range(line_no)` 原始字节 `read_window(start, len)` 写 `writer`；每行判 `cancel`；每 4096 行 `on_progress(written)`；读/写 `Err` → `errored=true` 停止（已写保留）。

- [ ] **Step 1：写失败测试**：
  - `export_selected_lines_byte_parity`：夹具 `"a\nbb\nccc\n"`，matcher=include 字面 "b"→ 保留行 `[1]`（"bb"）；写入手 `Vec<u8>`；断言 bytes == `&b"bb\n"[..]`（含换行）；`written==1,scanned==3`。
  - `export_preserves_crlf_and_gbk_bytes`：GBK 编码（用 `encoding::encode_all(Gbk,...)` 生成）多行，导出全部（空条件？取 include 一恒定串）→ 目标字节 == 源全文件字节（逐字节）。
  - `export_cancel_stops_early`：`cancel` 第 2 次真 → `cancelled==true`、`written <` 全量、已写为前缀。
  - `last_line_no_terminator`：夹具末行无 `\n` → 导出该行字节不含多余 `\n`、与源尾字节一致。
- [ ] **Step 2：** `cargo test -p lumelog-core export::` → FAIL。
- [ ] **Step 3：实现** `export.rs::run`（见 Interfaces）。用 `for (line_no, text) in 分块行`；保留则 `let (s,e)=doc.line_byte_range(line_no)` → `let bytes = doc.read_window(s, (e-s) as usize)?` → `writer.write_all(&bytes)`。`?` 错误在函数内 catch 为 `errored` 早停（不向上传 `Result`）。
- [ ] **Step 4：** `cargo test -p lumelog-core` PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:core-export-lines-raw-bytes` + push。

---

## Task 3: 时序/导出 DTO + 命令入参（`lumelog-app`）

**Files:**
- Modify: `app/src-tauri/src/dto.rs`
- Test: `app/src-tauri/src/dto.rs`（`#[cfg(test)] mod timeline_export_dto_tests`）

**Interfaces:**
- Consumes: `lumelog_core::filter::{TimeSpec, TimeFormat}`。
- Produces（camelCase serde）：
  - `pub struct TimeSpecJson { pub extract_pattern: String, pub format: String, pub from: Option<String>, pub to: Option<String>, pub keep_when_no_ts: bool }`（serde camelCase：`extractPattern/format/from/to/keepWhenNoTs`），`to_core(self) -> TimeSpec`（`format` `"iso"|"epoch_s"|"epoch_ms"` → `TimeFormat`，未知默认 `Iso`）。
  - `pub struct ExportProgress { written: u64 }`；`pub struct ExportStatsJson { written:u64, scanned:u64, cancelled:bool, errored:bool }`（core `ExportStats` → Json 或 `From`）。
  - `filter_run` 命令体已在 Task 4 改；此任务仅定义 DTO。
- [ ] **Step 1：写失败测试**：`time_spec_json_deserializes_and_maps`（`{"extractPattern":"(?<ts>.)","format":"epoch_ms","from":null,"to":"5","keepWhenNoTs":true}` → `to_core` 得 `EpochMillis`、`to=Some("5")`）；`export_stats_json_camel_case`（键 `written/scanned/cancelled/errored`）。
- [ ] **Step 2：** `cargo test -p lumelog-app timeline_export_dto` → FAIL。
- [ ] **Step 3：实现** 两 DTO + `to_core`；core `ExportStats`→`ExportStatsJson` 的 `From`。未接入前对 `ExportProgress`/`ExportStatsJson` 加带注释 `#[allow(dead_code)]`（Task 5 剥）。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:timeline-export-dto` + push。

---

## Task 4: `filter_run` 接 time/line_range + 更新 `scan_filter`

**Files:**
- Modify: `app/src-tauri/src/commands.rs`
- Test: `app/src-tauri/src/commands.rs`（`filter_run_tests` 追加 + 修 `scan_filter`/`keep` 调用）

**Interfaces:**
- Consumes: `LineFilter::build(specs, case, time, line_range)`（Task 1 新签名）、`TimeSpecJson::to_core`。
- Produces：`filter_run` 入参加 `time: Option<TimeSpecJson>, line_from: Option<u64>, line_to: Option<u64>`；`scan_filter` 的 `lf.keep(&text)` → `lf.keep(line1-1, &text)`。

- [ ] **Step 1：改现有 + 写失败测试**：现有 `filter_run_tests` 的 `make_filter`/`scan_filter` 调用适配新 `build`（补 `, None, None`）与 `keep(line_no,...)`；追加 `scan_filter_time_axis_keeps_range`（构造带 `time` 的 `LineFilter`，夹具含 ISO 时间行，断言保留集仅区间内行号）、`scan_filter_line_range_degrade`（`build(...,None,Some((2,3)))` → 仅第 2、3 行保留）。
- [ ] **Step 2：** `cargo test -p lumelog-app filter_run` → FAIL（编译期即缺新签名）。
- [ ] **Step 3：实现**：`filter_run` 组 `specs` 后 `let line_range = match (line_from, line_to) { (Some(a),Some(b)) => Some((a,b)), _ => None };`；`let time = time.map(|t| t.to_core());`；`LineFilter::build(specs, case_sensitive, time, line_range).map_err(from_core)?`（空 `specs` 且无 `time` 且无 `line_range` 仍视为 no-op 早返 `{0,0,false}`——沿用增量1 守卫，但注意"仅时序/仅行号"是有效查询、不 no-op）。`scan_filter` 逐行 `keep(line1-1,&text)`。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + `cargo clippy --workspace --all-targets -- -D warnings` 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:filter-run-timeline-axis` + push。

---

## Task 5: `export_lines` / `export_cancel` 命令

**Files:**
- Modify: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/lib.rs`（注册 `commands::export_lines, commands::export_cancel`）
- Test: `app/src-tauri/src/commands.rs`（`#[cfg(test)] mod export_cmd_tests`）

**Interfaces:**
- Consumes: `lumelog_core::export::run`、`LineFilter::build`、`state::{begin_search,end_search,cancel,get}`、`AppHandle`+`Manager`、`std::fs::File`、`std::io::BufWriter`、`Channel`。
- Produces：
  - `#[tauri::command] #[allow(clippy::too_many_arguments)] pub fn export_lines(state, app: AppHandle, doc_id: u64, conditions: Vec<FilterCondition>, case_sensitive: bool, time: Option<TimeSpecJson>, line_from: Option<u64>, line_to: Option<u64>, target_path: String, on_progress: Channel<ExportProgress>, on_done: Channel<ExportStatsJson>) -> Result<u64, IpcError>`
  - `#[tauri::command] pub fn export_cancel(state, request_id: u64) -> Result<(), IpcError>`
  - 抽纯函数 `fn open_export_target(path: &str) -> Result<std::fs::File, IpcError>`（io 错映射 `io`）便于测；worker 体测走 `export::run`（Task 2 已测），此处测命令薄封装能编译 + `export_cancel` 命中/未命中。
- [ ] **Step 1：写失败测试**：`export_cancel_unknown_request`（对未注册 id 返 `unknown_request`）；`open_export_target_bad_path_is_io`（`Z:/no/valid` → code `io`）。
- [ ] **Step 2：** `cargo test -p lumelog-app export_cmd` → FAIL。
- [ ] **Step 3：实现**：prologue 同 `filter_run`（校验 doc、`LineFilter::build`、`File::create(target_path)` 失败即 `fail(io)`）→ `spawn_blocking(move || { let mut w = BufWriter::new(file); let stats = export::run(&mut doc_for_task, &matcher, &mut w, &mut |written| { let _ = on_progress.send(ExportProgress{written}); }, &cancelled); let _ = w.flush(); let _ = on_done.send(stats.into()); app.state::<AppDocs>().end_search(req_id); })`；`Ok(req_id)`。worker 需 `DocFile`：在 prologue `state.get(doc_id)` 取 `doc_arc`，任务内 `DocLines`-式访问——`export::run` 收 `&mut DocFile`，故任务内 `doc_arc.lock()` 取 guard（`unwrap_or_else(poison.into_inner())`）整任务持有（后台线程持锁可接受：`filter_run`/`can_parse` 亦是 worker 持锁扫描；UI 不冻结）。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + workspace clippy 净。
- [ ] **Step 5：** CHANGELOG（记 worker 持 `doc` 锁扫描的取舍，与增量1 `can_parse` 一致）；commit `feat:export-lines-command` + push。

---

## Task 6: 前端 IPC 桥（`ipc.ts`）

**Files:**
- Modify: `app/frontend/src/ipc.ts`
- Test: `app/frontend/src/ipc.test.ts`

**Interfaces:**
- Produces：
  - `export type TimeFormat = "iso" | "epoch_s" | "epoch_ms";`
  - `export interface TimeSpec { extractPattern: string; format: TimeFormat; from?: string; to?: string; keepWhenNoTs: boolean }`
  - `export interface ExportStats { written: number; scanned: number; cancelled: boolean; errored: boolean }`
  - `filterRun` args 增 `time?: TimeSpec; lineFrom?: number; lineTo?: number;`（透传 `time/line_from/line_to`）。
  - `exportLines(args: { docId; conditions; caseSensitive; time?; lineFrom?; lineTo?; targetPath: string; onProgress: (n:{written:number})=>void; onDone: (s: ExportStats)=>void }): Promise<number>`
  - `exportCancel(requestId: number): Promise<void>`
- [ ] **Step 1：写失败测试**：`exportLines`/`exportCancel` typeof function；`filterRun` 透传 `time`/`lineFrom`/`lineTo`（mock `invoke` 断言 payload 键名 camelCase）。
- [ ] **Step 2：** `npm --prefix app/frontend test` → FAIL。
- [ ] **Step 3：实现** 上述类型 + 函数 + `filterRun` 扩参。
- [ ] **Step 4：** `npm test` PASS + `run build` tsc PASS。
- [ ] **Step 5：** CHANGELOG；commit `feat:ipc-timeline-export-bridge` + push。

---

## Task 7: `main.ts` 查询面板时序区 + 导出流程

**Files:**
- Modify: `app/frontend/src/main.ts`
- Test: `app/frontend/src/main.ts` 内导出一个纯函数 `assembleTimeSelection(...)` 并在 `main.test.ts`？——`main.ts` 无测试文件；把可判定逻辑抽为导出纯函数 `pickTimelineInput(raw): { time?: TimeSpec; lineFrom?: number; lineTo?: number; error?: string }` 放入新 `app/frontend/src/query.ts` + `query.test.ts`。

**Interfaces:**
- Consumes: `filterRun`（新参）、`exportLines/exportCancel`、`ask`（已 import from plugin-dialog）、`open`/`save`。
- Produces：新模块 `app/frontend/src/query.ts`：`export function pickTimelineInput(extractPattern: string, format: TimeFormat, from: string, to: string, keepWhenNoTs: boolean): { time?: TimeSpec; lineFrom?: number; lineTo?: number; error?: string }`（`extractPattern` 非空→time（from/to 空转 undefined）；否则 from/to 非空→解析为整数行号（非整数/`from>to` 置 `error`）；都空→`{}`）。`main.ts` 面板加时序控件 + 导出按钮。

- [ ] **Step 1：写 `query.test.ts` 失败测试**：`extract_present_sends_time`、`extract_blank_sends_line_range`、`line_range_non_integer_errors`、`from_gt_to_errors`、`empty_is_noop`。
- [ ] **Step 2：** `npm test` → FAIL（`query.ts` 未定义）。
- [ ] **Step 3：实现 `query.ts`**（纯函数）。
- [ ] **Step 4：`main.ts` 集成**：面板加"时序"区（抽取正则 input + 格式 select + 起/止 input + 保留无 ts 勾选）；`applyQuery` 用 `pickTimelineInput` 得 `time/lineFrom/lineTo` 传 `filterRun`（错误 → `toastMsg` 拒）。加"导出"按钮 → 取当前条件+时序 → 若"无有效条件且无时序/行号"→ `ask` 二次确认；`save` 对话框选路径；`exportBusy` 复用 `queryBusy` 单任务互斥；`queryReqId` 接 `exportCancel`；`onProgress` 更新状态"已导出 N 行…"；`onDone` 依 `cancelled/errored` toast。`registerShortcuts` 无需改（Ctrl+Shift+F 已开面板）。
- [ ] **Step 5：** `npm test` PASS + `run build` tsc PASS。
- [ ] **Step 6：** CHANGELOG；commit `feat:query-panel-timeline-export` + push。

---

## Task 8: 样式 + 全量四门 + 终审 CodeReview + 合 main

**Files:**
- Modify: `app/frontend/src/styles.css`（`.query-time` 时序区、与增量1 面板对齐）
- Modify: `docs/claude.md`（合入后把"日志行过滤、高亮标记、时序筛选"勾选 + "导出筛选后的报文/日志片段"的行过滤导出部分注记完成；CAN `export_csv` 仍待）

- [ ] **Step 1：** 加 `.query-time` 样式（输入/select 排布，窄窗换行不溢）。
- [ ] **Step 2：** `npm run build` + `npm test` PASS。
- [ ] **Step 3：全量四门**：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`npm --prefix app/frontend test`、`npm --prefix app/frontend run build` → 全绿。
- [ ] **Step 4：GUI 手工清单**（`tauri dev`）：spec §7 时序/导出用例 1–5。
- [ ] **Step 5：** CHANGELOG 收尾；commit `style:query-panel-timeline` + push。
- [ ] **Step 6：** 派一个 CodeReview 审整条 `feat/m3-timeline-export`（`--no-ff` 前全分支），一个修复波 + 定向复审；清干净后 `--no-ff` 合 `main`、push、更新 `docs/claude.md` 里程碑、删本计划 SDD 工作区（`plan5`）。

---

## 备注：与 spec/增量1 的接口涟漪（决策）
- `LineFilter::build` 加 `time`/`line_range` 两参、`keep` 加 `line_no` 参（Task 1）→ 增量1 调用点（`commands.rs::scan_filter`、core `filter.rs` 现有单测）在 Task 1/4 同步更新，行为对无时序/无行号的旧查询完全等价。
- 空查询守卫调整：`filter_run` 仅当"无 `specs` 且无 `time` 且无 `line_range`"才 no-op（Task 4），因"仅时序""仅行号区间"是有效选中集。
- `export::run` 落在 core 是为触达 `pub(crate) line_byte_range/read_window`；worker 持 `doc` 锁整任务扫描（与 `can_parse`/`filter_run` 同构，UI 不冻结）。
