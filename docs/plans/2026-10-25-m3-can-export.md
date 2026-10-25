# M3 子项目 A · 增量3：CAN export_csv / export_jsonl Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 后端把一份 CAN 日志按 CanPanel 视图过滤**重新解码**并**流式**导出为宽表 CSV（meta 列 + 每个 DBC 信号一列、物理值）与 JSONL（每帧对象），带进度与取消；CanPanel dock 加"导出 CSV…/JSONL…"。

**Architecture:** 新 `app/src-tauri/src/can_export.rs` 收纯逻辑（`frame_matches` 逐字端口 `canfilter.ts` 作用在 `FrameJson`；`csv_header`/`csv_row`/`jsonl_line` 纯文本构造；`export_can` 扫描核）。`can_parse` 的"按 `source_kind`/`regex` 建 `FrameSource`"抽为共享 `build_frame_source`。命令 `export_csv`/`export_jsonl`/`export_can_cancel` 走 `export_lines` 同款异步壳。前端 dock 工具条加两按钮，读现有 `canSrcSel/canRegexInput/lastDbcId/canFilters`，与解析共用单飞互斥。

**Tech Stack:** Rust（`lumelog-app` + `lumelog-can`：`FrameSource`/`decode_signals`/`Database`/`frame_json`）+ `serde_json`（手写 CSV 转义，不引 `csv` 依赖以守依赖面）。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm --prefix app/frontend test` / `run build`（tsc）。

**Spec:** `docs/specs/2026-10-25-m3-can-export-design.md`。承接增量1/2（`can_parse`、`export_lines`/`export::run`、`canfilter.ts`、`FrameJson`/`frame_json`）。

## Global Constraints

- 简体中文 UI；DTO `#[serde(rename_all="camelCase")]` + `Option` `skip_serializing_if`。
- 不整文件驻留：`export_can` 逐帧 `next_frame`+解码+写，不缓冲全帧；CSV 列集仅来自 DBC（导入时已定）。
- `frame_matches` **必须与 `canfilter.ts` 语义逐条一致**（ID 闭/反、msgName 精确、信号五运算 `Eq` 精确 `f64`、时间窗缺 ts 不匹配、缺省轴不应用）。
- 命令异步同 `can_parse`/`export_lines`：`fn`+`spawn_blocking`，prologue 校验/建文件（坏路径同步 `io`），worker 持 `doc_arc.lock()` 整轮、`export_can_cancel` 走独立取消注册表、`on_done` 必发、`end_search` 每路径恰一次、早返 `req_id`。
- 逐任务 TDD（red→green）；每任务四门全绿再提交；commit 前追加根 `CHANGELOG.md`（时间倒序）；commit 消息无空格；commit 后 push；分支 `--no-ff` 合 `main`。
- 分支基：从 `main`（当前 `704df77`）新建 `feat/m3-can-export`。

## Review Focus

1. **`frame_matches` 与 `canfilter.ts` 漂移**：把 `canfilter.test.ts` 7 例逐字搬成 Rust 共享向量钉死。
2. **CSV 列名限定与顺序确定性**：`消息名.信号名`、按 `Database.messages`→`signals` 顺序；同名去重加 `#n`；帧未匹配消息时信号列全空。
3. **浮点稳定 ASCII 格式**：物理值不受 locale 影响、Excel 可解析（`{:.6}` 裁尾零）。
4. **流式 + 取消 + errored**：不缓冲全帧、每帧判 cancel、坏路径同步 `io` 不空写、`on_done` 必发、`end_search` 一次。
5. **与解析互斥**：解析进行中禁导出、导出进行中禁解析/再导出（dock 复用单飞态）。

---

## Task 1: `can_export` 纯逻辑：`frame_matches`（`canfilter.ts` 端口）

**Files:**
- Create: `app/src-tauri/src/can_export.rs`（`CanFilterJson`/`SignalOp` 移入或引自 dto；`frame_matches`；`#[cfg(test)] mod tests`）
- Modify: `app/src-tauri/src/dto.rs`（`CanFilterJson` + `SignalOp` serde camelCase，Option `skip_serializing_if`）
- Modify: `app/src-tauri/src/lib.rs`（`mod can_export;`）
- Test: `can_export.rs`（逐字移植 `canfilter.test.ts` 7 例）

**Interfaces:**
- Consumes: `crate::dto::{FrameJson, SignalValueJson}`（FrameJson 有 `can_id:u32`、`msg_name:Option<String>`、`timestamp_ms:Option<f64>`、`signals:Vec<SignalValueJson{name,physical,..}>`）。
- Produces: `pub enum SignalOp { Gt, Lt, Ge, Le, Eq }`（serde：`">"`/`"<"`/`">="`/`"<="`/`"=="`）；`pub struct CanFilterJson { id_from:Option<u32>, id_to:Option<u32>, msg_name:Option<String>, signal_name:Option<String>, signal_op:Option<SignalOp>, signal_value:Option<f64>, time_from_ms:Option<f64>, time_to_ms:Option<f64> }`（camelCase）；`pub fn frame_matches(fj:&FrameJson, f:&CanFilterJson) -> bool`。

- [ ] **Step 1：写失败测试**（`can_export.rs` tests，构造 `FrameJson` 助手）：`id_closed_range` / `id_reversed_matches_none` / `msg_name_exact` / `signal_gt` / `signal_eq_exact_nan` / `signal_missing_excludes` / `time_window_no_ts_excludes` / `empty_filter_keeps_all` —— 逐条对齐 `canfilter.test.ts`。
- [ ] **Step 2：** `cargo test -p lumelog-app can_export::` → FAIL（未定义）。
- [ ] **Step 3：实现** `frame_matches`（§4 语义：`id_from`/`id_to` 闭区间 + 反向不匹配；`msg_name` 需相等；信号三要素齐才判、`Eq` 用 `f64` 精确相等且 `value.is_nan()` 时不匹配、缺该信号不匹配；时间窗任一存在则缺 `timestamp_ms` 不匹配）；`SignalOp`/`CanFilterJson` 入 `dto.rs`，`lib.rs` 加 `mod can_export;`。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:can-export-frame-matches-port` + push。

---

## Task 2: 抽 `build_frame_source`，`can_parse` 改调用

**Files:**
- Modify: `app/src-tauri/src/commands.rs`（把 can_parse 内 match 源构造抽 `pub(crate) fn build_frame_source(kind:&str, regex:&Option<String>, lines:DocLines) -> Result<Box<dyn FrameSource>, IpcError>`；worker 内改用之；prologue 保留一次性 `RegexSource::new(empty,p)` 预编译校验与 kind 合法性、regex 非空校验）

**Interfaces:**
- Produces: `build_frame_source(kind:&str, regex:&Option<String>, lines:DocLines) -> Result<Box<dyn FrameSource>, IpcError>`（"asc"→AscSource、"regex"→RegexSource(正则须已 prologue 校验)、else candump）；行为与现 can_parse 内联分支逐字一致。
- `can_parse` 签名/语义不变（回归由既有 CAN 测 + `scan_filter`/`parse_frames` 测保证）。

- [ ] **Step 1：写失败测试**：无新单测（`build_frame_source` 需 `DocLines`/真实文件，行为等价重构）；以**编译期 + 既有 `can_parse` 相关测不回归**为门。加一测：`build_frame_source` 对未知 kind 返回 `source_format`（用一个 `DocLines::new` over tempfile，或把 kind/regex 校验保持纯：新增 `fn validate_source_kind_regex(kind,regex) -> Result<(),IpcError>` 纯函数，测它 3 例：未知 kind、regex 源缺 pattern、非法正则经一次性编译 → 对应码）。
- [ ] **Step 2：** `cargo test -p lumelog-app validate_source` → FAIL。
- [ ] **Step 3：实现** `validate_source_kind_regex`（纯，prologue 用它替代内联校验）+ `build_frame_source`（worker 用），`can_parse` 改调用（regex 在 `build_frame_source` 内 `expect("validated")`）。
- [ ] **Step 4：** `cargo test -p lumelog-app` 全绿（既有 CAN 测不回归）+ clippy 净。
- [ ] **Step 5：** CHANGELOG（记 `can_parse` 行为不变的重构）；commit `refactor:extract-build-frame-source` + push。

---

## Task 3: `can_export` 纯文本构造：CSV 宽表 + JSONL 行

**Files:**
- Modify: `app/src-tauri/src/can_export.rs`（新增 `csv_columns`/`csv_header`/`csv_row`/`jsonl_line` + 浮点格式化 + CSV 转义）
- Test: `can_export.rs`（构造 `FrameJson` + 内联小 `Database`/`Message`/`Signal` 断言文本）

**Interfaces:**
- Consumes: `FrameJson`、`lumelog_can::dbc::Database`（`messages[].{name, signals[].name}`）。
- Produces:
  - `pub fn csv_columns(db:Option<&Database>) -> Vec<String>`（无 db 空；否则按序 `消息名.信号名`，重名 `#n`）。
  - `pub fn csv_header(db:Option<&Database>) -> String`（meta 固定 7 列 + `csv_columns`，CRLF 结尾、`"` 转义）。
  - `pub fn csv_row(fj:&FrameJson, col_index:&HashMap<String,usize>, ncols:usize) -> String`（meta + 按 `fj.msg_name`+`signal.name` 定位填物理值，其余空；CRLF）。
  - `pub fn jsonl_line(fj:&FrameJson) -> String`（`serde_json::to_string(fj)`+`\n`）。
  - `fn fmt_physical(v:f64)->String`（`{:.6}` 裁尾零；`NaN`/`inf` → 空串或固定串，定案）。

- [ ] **Step 1：写失败测试**：`csv_columns_qualified_and_ordered`（两消息、含重名信号→`#2`）；`csv_header_meta_then_signals`；`csv_row_fills_matched_signals_blank_other_cols`（给定两帧断言行文本，含逗号消息名转义 `"..，“"`）；`jsonl_line_is_valid_json_with_signals`（`serde_json::from_str` 回读）；`fmt_physical_stable`（`512.0→"512"`、`50.25→"50.25"`、`0.1+0.2→"0.3"` 或既定裁尾规则）。
- [ ] **Step 2：** `cargo test -p lumelog-app can_export::tests::csv` → FAIL。
- [ ] **Step 3：实现** 四个构造器 + 转义（字段含 `,`/`"`/`\n` 才加引号、内部 `"`→`""`）+ 列索引 `HashMap`（在 export 侧构建一次传入）。`fmt_physical`：`NaN`/`inf`→空串定案。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:can-export-csv-jsonl-builders` + push。

---

## Task 4: `export_can` 扫描核

**Files:**
- Modify: `app/src-tauri/src/can_export.rs`（`export_can` + `CanExportStats`）
- Test: `can_export.rs`（tempfile 建 `DocFile` + 内联 `Database`，断言写出字节 + 取消 + errored）

**Interfaces:**
- Consumes: `DocFile`、`DocLines::new(Arc<Mutex<DocFile>>)`、`build_frame_source`（Task 2）、`frame_json`、`frame_matches`、`csv_header/csv_row/jsonl_line`、`Channel` 无关（回调注入）。
- Produces: `pub struct CanExportStats { written:u64, scanned:u64, cancelled:bool, errored:bool }`；`pub fn export_can<W:Write>(doc:&mut DocFile, dbc:Option<&Database>, kind:&str, regex:&Option<String>, filter:&CanFilterJson, writer:&mut W, on_progress:&mut dyn FnMut(u64), cancel:&dyn Fn()->bool, wide:bool) -> CanExportStats`（wide=true 走 CSV(header+rows)，false 走 JSONL；逐 `next_frame`→`frame_json`→`frame_matches`→写；每帧判 cancel；每 512 批 on_progress；读/写错→`errored` 停止）。
- 注：`export_can` 需 `&mut DocFile`；建源用 `DocLines::new` 需 `Arc<Mutex<DocFile>>`——故签名改吃 `&Arc<Mutex<DocFile>>` 并在内部锁一次（与命令体一致），或命令体持 guard 传 `&mut DocFile` 而 `build_frame_source` 的 `DocLines` 另锁——**定案**：`export_can(doc:&mut DocFile,...)`，内部用 `chunk_at` 逐块造 `DocLines`-等价行（复用 `DocLines` 需要 Arc<Mutex>）。为避免双锁，命令体把 `Arc<Mutex<DocFile>>` clone 进 worker、`export_can` 签名改 `doc_arc:&Arc<Mutex<DocFile>>` 并在首行 `let mut doc = doc_arc.lock()` 持锁至返回（同 `export::run` 模型，只是这里锁在函数内）。计划取**后者**：`export_can(doc_arc, dbc, ...)`，函数内锁 guard 贯穿。

- [ ] **Step 1：写失败测试**：`export_can_csv_writes_header_and_filtered_rows`（tempfile candump 文本 + 内联 Database，wide=true → 断言字节含 header + 命中行、含/不含信号列按 filter）；`export_can_jsonl_lines`（wide=false，逐行 `from_str` 合法、计数）；`export_can_cancel_stops`（cancel 第 N 真 → `cancelled`、written<全量、输出为前缀）。
- [ ] **Step 2：** `cargo test -p lumelog-app export_can` → FAIL。
- [ ] **Step 3：实现**（锁 guard + build_frame_source + 逐帧 + wide 分支 + 列索引预构建）。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:can-export-scan-core` + push。

---

## Task 5: `export_csv`/`export_jsonl`/`export_can_cancel` 命令 + 注册

**Files:**
- Modify: `app/src-tauri/src/commands.rs`（三命令薄壳）
- Modify: `app/src-tauri/src/lib.rs`（注册）
- Test: `commands.rs`（`export_cmd_tests` 追加）

**Interfaces:**
- Produces: `#[tauri::command] export_csv(...)`、`export_jsonl(...)`（同 §5 签名；prologue：`get(doc)`→`unknown_doc`、`validate_source_kind_regex`、`dbc_id.and_then(get_dbc)`、`open_export_target(target_path)` 复用现成 io helper、`spawn_blocking` 内 `export_can(&doc_arc, dbc, &kind, &regex, &filter, &mut BufWriter::new(file), &mut prog, &cancelled, wide)` → flush → `on_done.send(stats)` → `end_search` → 早返 req_id）；`export_can_cancel`（转发 `state.cancel`）。`CanExportProgress{written}`、`CanExportStatsJson{written,scanned,cancelled,errored}`(+`From<can_export::CanExportStats>`)。
- [ ] **Step 1：写失败测试**：`export_can_cancel_unknown_request`?（需 State，跳）；测薄壳可测点：`CanExportStatsJson` From 映射 + camelCase（已有同类，补 `can_export_stats_json_camel_case`）。命令体由 `export_can` 测覆盖。
- [ ] **Step 2：** `cargo test -p lumelog-app` → FAIL（新类型/命令未定义时编译失败即红）。
- [ ] **Step 3：实现** 两命令 + cancel + DTO + 注册；worker 持锁经 `doc_arc`（`export_can` 内部锁）。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + `cargo clippy --workspace --all-targets -- -D warnings` 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:can-export-commands` + push。

---

## Task 6: 前端 `ipc.ts` 桥

**Files:**
- Modify: `app/frontend/src/ipc.ts`；Test: `app/frontend/src/ipc.test.ts`
**Interfaces:** Produces `CanExportStats` 类型 + `exportCsv`/`exportJsonl`（args `{docId,dbcId?,sourceKind,regex?,filter:CanFilter,targetPath,onProgress:(p:{written:number})=>void,onDone:(s:CanExportStats)=>void}: Promise<number>`，多 Channel）+ `exportCanCancel(requestId)`；`filter` 用 canfilter.ts 的 `CanFilter`（字段与 `CanFilterJson` camelCase 对齐）。
- [ ] **Step 1：写失败测试**：三函数 typeof；`exportCsv` mock `invoke` 断言 payload 键 `docId/dbcId/sourceKind/regex/filter/targetPath/onProgress/onDone`。
- [ ] **Step 2：** `npm test` FAIL → **Step 3：实现** → **Step 4：** `npm test` PASS + `run build` tsc PASS → **Step 5：** CHANGELOG；commit `feat:ipc-can-export-bridge` + push。

---

## Task 7: dock 导出按钮 + `main.ts` 流程

**Files:** Modify `app/frontend/src/main.ts`（dock 工具条 + `onCanExport(kind)`；无 canpanel.ts 改动）
**Interfaces:** 复用 `canSrcSel`/`canRegexInput`/`lastDbcId`/`canFilters.get(docId)`/`docStates`/`basenameOf`；新 `canExportBusy`/`canExportReqId`；与解析互斥（`canParsing||canExportBusy` 时"取消/再操作"提示；解析按钮进行中禁用导出，反之亦然）。`save` 对话框默认 `<docname>.csv`/`.jsonl`；进度更新状态/按钮；完成 toast 区分 `cancelled/errored/成功`；"取消"调 `exportCanCancel(canExportReqId)`。
- [ ] **Step 1：实现** dock 加"导出 CSV…""导出 JSONL…"两按钮 + `onCanExport`；复用 `applyFilterFromInputs` 后的 `canFilters` 当前值作 `filter`；互斥/进度/取消/toast。
- [ ] **Step 2：** `npm test` 不回归 + `run build` tsc PASS。
- [ ] **Step 3：** 手工冒烟（能 `tauri dev` 则验，否则记 Task 8 清单）。
- [ ] **Step 4：** CHANGELOG；commit `feat:dock-can-export-buttons` + push。

---

## Task 8: 全量四门 + 终审 CodeReview + 合 main

- [ ] **Step 1：全量四门**：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`npm --prefix app/frontend test`、`npm --prefix app/frontend run build` → 全绿。
- [ ] **Step 2：手工验收清单**（`tauri dev`）：spec §9 GUI 全过。
- [ ] **Step 3：** 派 CodeReview 审整条 `feat/m3-can-export`（`--no-ff` 前全分支，diff 包 + spec/plan + 上述约束/Review Focus），一个修复波 + 定向复审；清干净后 `--no-ff` 合 `main`、push、更新 `docs/claude.md`（"导出筛选后的报文"勾选 CAN CSV/JSONL 完成）、删本计划 SDD 工作区。
- [ ] **Step 4：** `Cargo.lock`（若新增依赖）随首个引用它的提交一并 `git add`；合并后在 `main` 复跑 `cargo test -p lumelog-app` + `run build` 复验。

---

## 备注：与既有代码的接缝/决策
- 不新增 crate 依赖：CSV 手写转义；`serde_json = "1"` 已是 `lumelog-app` 普通依赖（已核实），`jsonl_line` 可直接 `serde_json::to_string`；无需改 Cargo.toml。
- `frame_matches` 作用 `FrameJson`：与前端零语义漂移（同结构），是端口风险的主动规避。
- 无 DBC：CSV 仅 meta、信号条件不匹配（与前端 signals 空一致）。
- dock 而非 CanPanel 承载导出按钮：过滤/源/dbc 态全在 `main.ts`，避免向 `CanPanel` 传回调的样板（YAGNI）。
