# M3 子项目 A · 增量1：只读 行过滤 + 高亮命中 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在只读大日志视图上，按多条"包含/排除 × 字面/正则"条件（AND）过滤命中行、对命中区间加背景高亮并可在命中间跳转，全程流式、可取消、不把整文件文本驻留内存。

**Architecture:** 后端 `lumelog-core` 新增纯函数行过滤器求值器；`lumelog-app` 新增 `filter_run`（异步 `spawn_blocking` + 早返 `req_id` + 流式批次 + 复用取消注册表，镜像已改造的 `can_parse`）与 `doc_lines_at`（按需取子集行文本）。前端 `listview.ts` 在既有 `visibleWindow` 虚拟化上新增"子集视图"（渲染命中行数组 + cu16 区间切 span），`main.ts` 把 `Ctrl+Shift+F` 搜索升级为统一"查询面板"（多条件行 + 过滤/高亮两开关 + F3 循环 + 停止）。

**Tech Stack:** Rust（`lumelog-core` 新 `filter` 模块 + `lumelog-app` 命令层 + `tauri::ipc::Channel`）、TypeScript 原生 DOM、`regex`、`memchr`。测试 `cargo test` / `vitest`，类型门 `tsc --noEmit`，静态门 `cargo clippy --workspace --all-targets -- -D warnings`。

**Spec:** `docs/specs/2026-10-25-m3-filter-highlight-design.md`（本计划据此论证）。

## Global Constraints

- UI 文案简体中文；DTO 结构体 `#[serde(rename_all = "camelCase")]`，可缺省字段用 `Option` + `#[serde(skip_serializing_if = "Option::is_none")]`。
- `char_map` **不出 IPC**：高亮区间在后端把字节跨度换算为**行内 UTF-16 码元偏移（cu16）**后再回传；前端 `String.slice` 按 cu16 直接切分（JS 字符串索引即 UTF-16 码元）。
- 不整文件驻留（主 spec §3）：`filter_run` 只回命中行号 + cu16 区间，不回行文本；文本按需经 `doc_lines_at` 取可见窗口。
- 行文本语义：过滤/高亮作用在**去掉行尾换行符的解码文本**上，与 `doc_lines_at`/既有 `loadWindow` 渲染的行文本逐字符一致（偏移才对得上）。
- 命令参数 camelCase↔snake_case 由 Tauri 自动转换。
- 逐任务 TDD（先写失败测试→跑红→实现→跑绿）；每任务四门全绿（`cargo test`、`cargo clippy --workspace --all-targets -- -D warnings`、`npm --prefix app/frontend test`、`npm --prefix app/frontend run build`）方可提交；commit 前追加根 `CHANGELOG.md`（时间倒序、写清文件/原因/验证）；commit 消息无空格无特殊字符（Windows shell）；commit 后直接 push；分支开发、`--no-ff` 合 main。
- 分支基：从 `main`（当前 `e48eaa0`）新建 `feat/m3-filter-highlight`。

## Review Focus（spec 隐含、无单任务测试覆盖、最易伤到使用者）

1. **多字节/astral 行的 cu16 区间边界**：`regex` 命中给字节跨度，换算 cu16 必须与前端渲染文本的 UTF-16 索引一致（GBK/emoji 混合），差一位即错切 span。
2. **空包含 + 仅排除**：包含集为空 ⇒ 包含侧恒真（仅按排除剔除），此时保留行无高亮区间（`ranges=[]`）仍应显示；包含与排除皆空 ⇒ 不发起扫描。
3. **取消与截断**：取消早停后 `on_done` 仍发（含已得计数）；命中区间数超 `HIT_RANGE_CAP` 时降级为空区间 + `truncated=true`，不 OOM；`end_search` 每路径恰好一次。
4. **过滤↔非过滤切换的窗口一致性**：`listview` 在 `hits.length` 与全量 `totalLines` 间切换时，`renderFirst/renderLast` 短路与缓存须失效重置，否则停在旧窗口/串位。
5. **字面 vs 正则大小写**：`case_sensitive=false` 字面走 Unicode 大小写无关包含、正则走 `(?i)`；字面含正则元字符不得被当模式。

---

## Task 1: lumelog-core 行过滤求值器（纯函数）

**Files:**
- Create: `crates/lumelog-core/src/filter.rs`
- Modify: `crates/lumelog-core/src/lib.rs`（`pub mod filter;`）
- Test: `crates/lumelog-core/src/filter.rs`（`#[cfg(test)] mod tests`，纯字符串无 IO）

**Interfaces:**
- Consumes: `regex::{Regex, RegexBuilder}`、`std::mem`（字面匹配可选用 `memchr`，允许退化为 `str::contains`）。
- Produces:
  - `pub enum CondMode { Include, Exclude }`
  - `pub enum CondKind { Literal, Regex }`
  - `pub struct CondSpec { pub mode: CondMode, pub kind: CondKind, pub text: String }`
  - `pub struct Cu16Range { pub start: u32, pub len: u32 }`（行内 UTF-16 码元）
  - `pub struct LineFilter { … }`，方法：
    - `pub fn build(specs: Vec<CondSpec>, case_sensitive: bool) -> Result<LineFilter, CoreError>`（编译各正则；字面空串/非法正则 → `CoreError::InvalidPattern`）
    - `pub fn has_conditions(&self) -> bool`
    - `pub fn keep(&self, line_text: &str) -> Option<Vec<Cu16Range>>`（`Some`= 保留，附各**包含**条件命中的 cu16 区间，可空；`None`= 剔除）。

- [ ] **Step 1：写失败测试**（`filter.rs` 内 `#[cfg(test)]`），覆盖：
  - `literal_include_keeps_matching_lines`：单条包含"错误"，含则 `keep` 返 `Some`（区间覆盖"错误"），否则 `None`。
  - `and_semantics_all_includes_required`：两条包含 "a"、"b"，仅含 "a" 的行 `None`，同含 "a""b" 的行 `Some` 且区间含两处。
  - `exclude_vetoes`：包含 "keep" + 排除 "drop"；"keep drop" 行 `None`，"keep ok" 行 `Some`。
  - `empty_include_only_exclude_vacuous_true`：仅一条排除 "x"，"abc" 行 `Some`（`ranges` 为空），"x..." 行 `None`。
  - `regex_case_insensitive`：正则 `Error` + `case=false` 命中 "error"/"ERROR"；`case=true` 仅 "Error"。
  - `literal_metachar_is_literal`：字面 `(a` 只匹配含字面 "(a" 的行，不作正则。
  - `utf16_offsets_for_multibyte`：行 `"错误 \u{1F600}x"`，包含 `\u{1F600}`（emoji，UTF-16 长度 2）：断言返回 `Cu16Range` 的 `start` 等于 emoji 前字符的 UTF-16 码元数、`len==2`（钉住字节→cu16 换算正确，Review Focus 1）。
  - `empty_pattern_rejected`：`build` 收字面空串 → `Err(InvalidPattern)`。
- [ ] **Step 2：** `cargo test -p lumelog-core filter:: ` → 预期 FAIL（模块/类型未定义）。
- [ ] **Step 3：实现 `filter.rs`**：`LineFilter` 持 `Vec<CompiledCond>`，`CompiledCond{mode, matcher}`；`matcher` 为 `Regex`（正则，`case_insensitive(!case)`，`unicode(true)`）或字面匹配器（`case` 真→`text.contains(pat)`；假→ `Regex::new(&regex::escape(pat))` 且 `case_insensitive(true).unicode(true)` 复用同一 `find_iter` 路径以产区间）。`keep`：对每包含条件 `find_iter(line_text)` 收 `Cu16Range`（`start = line_text[..m.start()].encode_utf16().count() as u32`，`len = line_text[m.start()..m.end()].encode_utf16().count() as u32`）；任一包含无匹配 → `None`；任一排除 `is_match` → `None`；否则 `Some(合并升序区间)`。空条件 `keep` 恒 `Some(vec![])`（由上层保证空条件不扫描）。
- [ ] **Step 4：** `cargo test -p lumelog-core` → PASS；`cargo clippy -p lumelog-core --all-targets -- -D warnings` → 净。
- [ ] **Step 5：** CHANGELOG 追加；commit `feat:core-line-filter-evaluator` + push。

---

## Task 2: 过滤 DTO + 错误映射（`lumelog-app`）

**Files:**
- Modify: `app/src-tauri/src/dto.rs`
- Test: `app/src-tauri/src/dto.rs`（`#[cfg(test)] mod filter_dto_tests`）

**Interfaces:**
- Consumes: `lumelog_core::filter::{CondSpec, CondMode, CondKind, Cu16Range}`；`IpcError`（`app/src-tauri/src/dto.rs` 内既有 `from_core`/`from_can`）。
- Produces（serde camelCase，供 `filter_run`/`doc_lines_at`）：
  - `pub struct FilterCondition { mode: String, kind: String, text: String }` → `to_core(self) -> CondSpec`（"include"/"exclude"、"literal"/"regex"，未知值走默认 include/literal）。
  - `pub struct Cu16RangeJson { cu16_start: u32, cu16_len: u32 }`
  - `pub struct LineHit { line: u64, ranges: Vec<Cu16RangeJson> }`
  - `pub struct FilterStats { kept: u64, scanned: u64, truncated: bool }`
  - `pub struct IndexedText { index: u32, text: String }`
  - `pub struct LineContentPayload { lines: Vec<IndexedText>, invalid_count: usize }`

- [ ] **Step 1：写失败测试**：`filter_condition_deserializes_camel_case`（JSON `{"mode":"include","kind":"regex","text":"a"}` → `to_core` 得 `CondSpec{Include,Regex,"a"}`）；`line_hit_serializes_camel_case`（`LineHit` serde 输出键为 `line`/`ranges`，`Cu16RangeJson` 键 `cu16Start`/`cu16Len`）；`filter_stats_shape`（键 `kept`/`scanned`/`truncated`）。
- [ ] **Step 2：** `cargo test -p lumelog-app` → 预期 FAIL。
- [ ] **Step 3：实现** 上述结构 + `to_core`（+ 未接入命令前对暂时未用项加带注释 `#[allow(dead_code)]`，Task 3 接入后剥除）。确认 `IpcError::from_core` 把 `CoreError::InvalidPattern` 映射为既有码；若无 `bad_regex`/`regex_compile` 对应，则新增映射：`InvalidPattern(_)` → code `"bad_regex"`（与 CAN `from_can` 同码串，前端复用既有中文表）。
- [ ] **Step 4：** `cargo test -p lumelog-app` + `cargo clippy -p lumelog-app --all-targets -- -D warnings` → PASS/净。
- [ ] **Step 5：** CHANGELOG；commit `feat:filter-dto-and-error-map` + push。

---

## Task 3: `filter_run` 命令（异步 + 流式 + 取消 + 截断）

**Files:**
- Modify: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/lib.rs`（`generate_handler!` 加 `commands::filter_run, commands::filter_cancel`）
- Test: `app/src-tauri/src/commands.rs`（`#[cfg(test)] mod filter_run_tests`）

**Interfaces:**
- Consumes: `AppDocs::{begin_search,end_search,cancel,get}`、`DocFile::{chunk_at,line_count,encoding}`、`lumelog_core::filter::LineFilter`、`tauri::async_runtime::spawn_blocking`、`tauri::ipc::Channel`、`AppHandle`+`Manager`（结束取回 `AppDocs` 做 `end_search`，同 `can_parse`）、`PARSE_BATCH`（512）。新增常量 `HIT_RANGE_CAP: usize = 50_000`。
- Produces:
  - `#[tauri::command] pub fn filter_run(state: tauri::State<'_, AppDocs>, app: AppHandle, doc_id: u64, conditions: Vec<FilterCondition>, case_sensitive: bool, on_batch: Channel<Vec<LineHit>>, on_done: Channel<FilterStats>) -> Result<u64, IpcError>`
  - `#[tauri::command] pub fn filter_cancel(state: tauri::State<'_, AppDocs>, request_id: u64) -> Result<(), IpcError>`（命中 `state.cancel` 否则 `unknown_request`）

- [ ] **Step 1：写失败测试**（直调 `filter_run` 需 `AppHandle`，headless 不可构造 → 把纯扫描逻辑抽为 `fn scan_filter(doc: &mut DocFile, lf: &LineFilter, emit: &mut dyn FnMut(Vec<LineHit>), cancelled: &dyn Fn() -> bool, cap: usize) -> FilterStats` 并测该函数，命令体只做"取 doc → spawn_blocking 调 scan_filter → on_done → end_search"）：
  - `scan_filter_and_keeps_correct_lines`：夹具文本（含 包含 "ERROR"、排除 "debug"），断言 kept 行号集合/升序、`scanned` 等总行数。
  - `scan_filter_emits_cu16_ranges`：对多字节行断言 `LineHit.ranges` 与 Task 1 换算一致。
  - `scan_filter_cancel_stops_early`：`cancelled` 第 N 行起真 → kept < 全量、仍回已得。
  - `scan_filter_truncates_ranges_over_cap`：`cap=2` 时第 3+ 命中行 `ranges` 空、`truncated=true`、仍计数 kept。
- [ ] **Step 2：** `cargo test -p lumelog-app filter_run` → FAIL。
- [ ] **Step 3：实现** `scan_filter` + `filter_run`（同步 prologue：`begin_search`、`get(doc_id)` 否则 `unknown_doc`、`LineFilter::build` 编译条件失败经 `IpcError::from_core`→`bad_regex` 直接 `fail` 返回；空条件 `on_done.send(kept0)` 前直接 `Ok(req_id)` 或 `invalid_argument`——取"空条件视为不扫描：`end_search` 后返回 `Ok(req_id)` 且 `on_done{0,0,false}`"，前端负责不无谓调用）；`spawn_blocking(move || { scan_filter(...); let _ = on_done.send(stats); app.state::<AppDocs>().end_search(req_id); })`，命令立即 `Ok(req_id)`。扫描逐行喂**去尾换行**的解码文本（复用与 `doc_lines`/CAN 同源的 chunk 切行，`LineFilter.keep` 得 `Some(ranges)` 则产 `LineHit`；每行判 `cancelled` 早停；`PARSE_BATCH` 聚合 `emit`）。`#[allow(clippy::too_many_arguments)]` 于 `filter_run`。`filter_cancel` 直接转发 `state.cancel`。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS；`cargo clippy --workspace --all-targets -- -D warnings` 净。
- [ ] **Step 5：** CHANGELOG（含 `filter_run` 异步决策 + 空条件处理 Ruling）；commit `feat:filter-run-async-stream` + push。

---

## Task 4: `doc_lines_at` 命令（按需取子集行文本）

**Files:**
- Modify: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/lib.rs`（注册 `commands::doc_lines_at`）
- Test: `app/src-tauri/src/commands.rs`（`#[cfg(test)] mod lines_at_tests`）

**Interfaces:**
- Consumes: `DocFile::chunk_at(line,1)`、`state.get(doc_id)`；`MAX_CHUNK_LINES`（20_000）。
- Produces: `#[tauri::command] pub fn doc_lines_at(state: tauri::State<'_, AppDocs>, doc_id: u64, indices: Vec<u32>, on_data: Channel<LineContentPayload>) -> Result<(), IpcError>`。抽纯函数 `fn gather_lines(doc: &mut DocFile, indices: &[u32]) -> Result<LineContentPayload, IpcError>` 便于测试（逐索引 `chunk_at(i,1)` 取文本、去尾换行，聚合 `IndexedText`；`invalid_count` 累加各行解码不可映射数；`indices.len() > MAX_CHUNK_LINES` → `invalid_argument`；doc 未知 → `unknown_doc`）。

- [ ] **Step 1：写失败测试** `gather_lines_returns_requested_subset`：夹具 `"a\nbb\nccc\n"`，`indices=[0,2]` → `lines=[{0,"a"},{2,"ccc"}]`（断言去尾换行、行号原值、升序）。`gather_lines_over_cap_rejects`：>cap → `invalid_argument`。（直调命令需 `State`/`Channel` → 测 `gather_lines` 纯函数，命令体仅转发。）
- [ ] **Step 2：** `cargo test -p lumelog-app lines_at` → FAIL。
- [ ] **Step 3：实现** `gather_lines` + `doc_lines_at`（对每个 index `chunk_at(index,1)`，`strip` 行尾 `\r\n`/`\n`；分组连续段为可选优化，非必须）。命令体：取 doc → `gather_lines` → `on_data.send(payload)`。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:doc-lines-at` + push。

---

## Task 5: 前端 IPC 包装 + 类型（`ipc.ts`）

**Files:**
- Modify: `app/frontend/src/ipc.ts`
- Test: `app/frontend/src/ipc.test.ts`（扩充）

**Interfaces:**
- Consumes: `invoke, Channel`；错误码联合 `IpcError` 已含 `bad_regex`/`regex_compile`/`source_format` 等（Task-4 of Plan3 已建）；本任务确认 `invalid_argument`/`unknown_request` 在联合与 `IPC_ERROR_ZH` 中（缺则补中文）。
- Produces：
  - `interface FilterCondition { mode: "include"|"exclude"; kind: "literal"|"regex"; text: string }`
  - `interface Cu16Range { cu16Start: number; cu16Len: number }`
  - `interface LineHit { line: number; ranges: Cu16Range[] }`
  - `interface FilterStats { kept: number; scanned: number; truncated: boolean }`
  - `interface IndexedText { index: number; text: string }`
  - `function filterRun(args: { docId: number; conditions: FilterCondition[]; caseSensitive: boolean; onBatch: (h: LineHit[]) => void; onDone: (s: FilterStats) => void }): Promise<number>`（双 `Channel`，镜像 `canParse`）
  - `function filterCancel(requestId: number): Promise<void>`
  - `function docLinesAt(args: { docId: number; indices: number[]; onData: (p: { lines: IndexedText[]; invalidCount: number }) => void }): Promise<void>`

- [ ] **Step 1：写失败测试**：`filterRun` 用 mock `invoke` 断言以 camelCase 传 `docId/conditions/caseSensitive` 且 `onBatch/onDone` 为 `Channel`、解析出 `req_id`；`filterCancel` 传 `requestId`；`IPC_ERROR_ZH` 对 `bad_regex`/`invalid_argument`/`unknown_request` 返回非空中文。
- [ ] **Step 2：** `npm --prefix app/frontend test` → FAIL。
- [ ] **Step 3：实现** 类型 + 三包装 + 补缺错误码中文。
- [ ] **Step 4：** `npm --prefix app/frontend test` + `run build`（tsc）PASS。
- [ ] **Step 5：** CHANGELOG；commit `feat:ipc-filter-bridge` + push。

---

## Task 6: `listview.ts` 子集视图（命中行数组渲染 + cu16 高亮）

**Files:**
- Modify: `app/frontend/src/listview.ts`
- Test: `app/frontend/src/listview.test.ts`（新增纯函数用例）

**Interfaces:**
- Consumes: 既有 `visibleWindow/ROW_HEIGHT/OVERSCAN`。
- Produces（导出纯函数便于测试）：
  - `export function sliceLineTextByRanges(text: string, ranges: Cu16Range[]): { text: string; hl: boolean }[]`——按 cu16 区间把一行切成片段数组（重叠/相邻区间先合并升序；越界区间忽略，Review Focus 5）。
  - `ListView` 新增可选"过滤模式"API：`setHits(hits: { line: number; ranges: Cu16Range[] }[] | null): void`（`null`=退出过滤，回连续 `loadWindow` 路径），及内部把"可见槽位 k → 原始行号 `hits[k].line`"映射、过滤模式下用注入的 `loadLines(indices: number[]): Promise<IndexedText[]>` 取文本、`paint` 对命中行按 `ranges` 调 `sliceLineTextByRanges` 生成 `<span class="line-hl">`。`ListViewOpts` 增 `loadLines?`。
  - `scrollToHit(hitIndex: number)`：过滤模式滚到 `hits[hitIndex]` 槽位（F3 跳转用）；`total()` 在过滤模式返回 `hits.length`。

- [ ] **Step 1：写失败测试**（`listview.test.ts`，无 DOM）：
  - `sliceLineTextByRanges_basic`：`"ERROR x"` + `[{0,5}]` → `[{text:"ERROR",hl:true},{text:" x",hl:false}]`。
  - `slice_merges_overlapping`：区间重叠合并，无重复片段。
  - `slice_ignores_out_of_range`：越界/负值区间被丢弃不 panic。
  - `slice_multibyte_uses_cu16`：含 astral 字符（`String.slice` 天然按 UTF-16）区间切分正确。
- [ ] **Step 2：** `npm --prefix app/frontend test` → FAIL。
- [ ] **Step 3：实现** `sliceLineTextByRanges`（合并升序去重叠 → 顺序切）+ `setHits`/`scrollToHit`/`loadLines` 子集渲染。关键：进入/退出 `setHits` 时重置 `renderFirst/renderLast = -1` 且清 `cache`（Review Focus 4）；过滤模式 `paint` 的行号槽位→原始行号映射，取文本走 `loadLines` 批量；高亮仅对**渲染文本**按 `ranges` 切 span（`whiteSpace:"pre"`）。
- [ ] **Step 4：** `npm --prefix app/frontend test` PASS（含既有用例不回归）+ `run build` tsc PASS。
- [ ] **Step 5：** CHANGELOG；commit `feat:listview-subset-view` + push。

---

## Task 7: `main.ts` 查询面板（替换旧搜索 UI）

**Files:**
- Modify: `app/frontend/src/main.ts`
- Test: 无新增单测（DOM/IPC 密集，`tauri dev` 手工 + `tsc` 为门）；确保编译期类型正确。

**Interfaces:**
- Consumes: `filterRun/filterCancel/docLinesAt`、`CanPanel` 无关；`lists`/`editors` Map、`tabs.active()`、`toastMsg`、`reportError`、`buildCommands`。
- Produces：查询面板（活动只读文档上）：
  - 状态：`let fltReqId: number|null`、`fltRunning: boolean`、`fltPanelEl`、条件行 DOM 列表、`hits: Map<docId, LineHit[]>`、当前 `hitCursor: Map<docId, number>`。
  - `openQueryPanel()`：构建/显示面板（`Ctrl+Shift+F` 由旧 `openSearchFlow` 改指向此；旧 `renderSearchShell/appendHits/hideSearchPanel` 移除或改造复用）。
  - `applyQuery()`：重入守卫 `if (fltRunning) return`；收集条件行→`FilterCondition[]`；空条件 → `toastMsg("请至少添加一条条件")` 不扫描；置 `fltRunning=true`、按钮转"停止"；`fltReqId = await filterRun({... onBatch: h=>hitsArr.push(...h); 若"过滤到命中行"开则 list.setHits(hitsArr); "高亮"开则叠加; onDone: s=>{计数/`s.truncated` 提示; fltRunning=false; 按钮复位}})`。"停止"→`filterCancel(fltReqId)`。
  - F3/Shift+F3：在当前 doc 的 `hits` 数组上前移/后移 `hitCursor`，调 `list.scrollToHit(cursor)`；仅只读文档生效。
  - `docLinesAt` 作为 `list` 的 `loadLines` 注入（`mountList` 时）。
  - `handleClose` 清 `hits`/`hitCursor` 映射。

- [ ] **Step 1：实现** 面板 DOM（条件行 增删、包含/排除 select、字面/正则 select、文本框、大小写勾选、过滤/高亮开关、上一个/下一个、停止、命中计数）+ 上述逻辑；`mountList` 注入 `loadLines: (idx)=> new Promise(res => docLinesAt({docId, indices: idx, onData: p=>res(p.lines)}))`；把 `Ctrl+Shift+F` 与命令面板 `search` 条目改指 `openQueryPanel`。
- [ ] **Step 2：** `npm --prefix app/frontend run build`（tsc）PASS；`npm test` 不回归。
- [ ] **Step 3：** 手工冒烟（能跑 `tauri dev` 则验，否则留待 Task 8 清单）：小日志含/排除/正则/大小写 → 过滤只显命中行 + 高亮 + F3 循环 + 停止。
- [ ] **Step 4：** CHANGELOG（记：查询面板取代旧搜索、旧 `search_run` 暂留但不再是主路径——Ruling）；commit `feat:query-panel-ui-filter-highlight` + push。

---

## Task 8: 样式 + 全量验收 + 终审

**Files:**
- Modify: `app/frontend/src/styles.css`（查询面板、条件行、`.line-hl`）
- Modify: `docs/claude.md`（M3 行过滤/高亮勾选与里程碑注记——仅在合入后更新）

**Interfaces:** 无新接口。

- [ ] **Step 1：实现样式**：`.line-hl`（命中背景 `var(--accent)` 低饱和或专用 token，三主题下均可辨）、查询面板 `#query-panel`（贴合 NoteLume 风 chrome，复用现有 token）、条件行按钮。
- [ ] **Step 2：** `npm --prefix app/frontend run build`（CSS 随 vite）+ `npm test` PASS。
- [ ] **Step 3：全量四门**：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`npm --prefix app/frontend test`、`npm --prefix app/frontend run build` → 全绿。
- [ ] **Step 4：手工验收清单**（`tauri dev`，headless 以单测+tsc 为门）：spec §7 GUI 用例 1–5 全过。
- [ ] **Step 5：** CHANGELOG 收尾；commit `style:query-panel-highlight-tokens` + push。
- [ ] **Step 6：** 派一个 CodeReview 子代理审整条 `feat/m3-filter-highlight`（`--no-ff` 前的全分支），一个修复波 + 定向复审；清干净后 `--no-ff` 合 `main`、push、更新 `docs/claude.md` 里程碑 + 删本计划 SDD 工作区。

---

## 备注：与 spec 的偏差（计划级决策）
- spec §3.3 "顺带把 `search_run` 异步化 + 接回 `search_cancel`" → **本增量不做**：查询面板统一由 `filter_run` 驱动（单条包含条件 = 原单模式搜索 + 高亮），旧 `search_run`/`search_cancel` 保留不删但退出主路径。理由：`filter_run` 已提供异步/流式/取消/多条件，再单独异步化 `search_run` 属重复劳动（YAGNI）；取消可用性由 `filter_run` 达成，spec §3.3 的动机（消除同步无取消的使用者可见短板）被满足。Ruling 记于 Task 7 CHANGELOG。
