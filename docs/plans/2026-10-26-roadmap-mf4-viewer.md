# Roadmap-4 增量1：MF4 测量数据查看器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 打开 `.mf4` → 元数据概览 + 通道搜索/多选 + 分页记录表 + 流式 CSV 导出（可取消）。

**Architecture:** `app/src-tauri/src/mf4.rs` 为唯一 `falcon_mdf` 边界；`state.rs` 增 `Mf4Sessions`；命令 `mf4_open/close/channels/summary/records/export_csv/export_cancel`；前端 MF4 dock（概览 / 通道栏 / `Mf4Table` 列式虚拟表）+ `mf4-ui.ts`，数据一律后端分页/窗口化。

**Tech Stack:** `falcon_mdf 0.7`（`default-features=false, features=["mmap"]`）、workspace MSRV `1.87 → 1.89`；TS 复用 `@tanstack/table-core`。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

**Spec:** `docs/specs/2026-10-26-roadmap-mf4-viewer-design.md`。分支基：`main`（当前 `a7ccba9`）→ `feat/roadmap-mf4-viewer`。

## Global Constraints

- 错误码：`mf4_not_mf4` / `mf4_encrypted` / `mf4_unsupported` / `io` / `invalid_argument` / `unknown_mf4`；全部有 `.zh` 文案。
- 上限：会话 ≤4；`channels.len() ≤ 32`；`limit ≤ 1000`（默认 500）；导出与取数必须分块，禁止整信号/整文件进内存。
- 排序：`timeAsc|timeDesc` 跨页全局；`col:<i>Asc|Desc` 仅本页（UI 标“本页排序”）。
- 单元格四类 `Number|Text|Bool|Null`；取不到值→`null`，该列 `degraded=true`。
- 时间窗 `[fromSec,toSec]`（秒，含端点）；`from>to` 前端拒；无时间通道 → `timeMs` 全 null + “无时间轴”。
- DTO camelCase；`mf4_export_*` 复用 `begin_search/end_search`；简体中文 UI；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. **边界纪律**：`falcon_mdf` 类型不得泄漏到 `mf4.rs` 之外；mmap 句柄生命周期（会话 drop 后文件可删除）。
2. **窗口正确性**：时间窗裁剪 + offset/limit 在排序后序列上分页；跨组/多源通道取数顺序与 `channels` 一致；`total_rows` 估算标注。
3. **值映射**：整数/浮点/布尔/文本/枚举转换、位域、无效样本 → `null` 且不 panic；大 `f64`（时间）在 JS 侧精度。
4. **导出**：表头/引号/换行、取消保留前缀、失败 `errored`、与表格内容一致。
5. **前端**：32 列上限提示、勾选→取数竞态（后发先至需丢弃旧响应）、翻页/时间窗联动、与 CAN dock 互斥、会话关闭清理。
6. **依赖面**：只开 `mmap`；MSRV 1.89 影响 CI 标注。

---

## Task 1: MSRV + 依赖 + 会话/元数据（`mf4_open/close/channels`）

**Files:** Modify `Cargo.toml`（`rust-version = "1.89"`）、`app/src-tauri/Cargo.toml`（`falcon_mdf = { version = "0.7", default-features = false, features = ["mmap"] }`）、`Cargo.lock`；Create `app/src-tauri/src/mf4.rs`；Modify `app/src-tauri/src/state.rs`（`Mf4Sessions`）、`lib.rs`（`mod mf4;` + 注册 `mf4_open/mf4_close/mf4_channels`）、`.github/workflows/ci.yml`（工具链注释 `>= 1.89`）
**Interfaces:** `pub(crate) struct Mf4Session { file: Arc<Mutex<Mf4File>>, channels: Vec<Mf4ChannelMeta> }`；`Mf4ChannelMeta { index, name, unit, kind, sync, group, source }`；`open_session(app, path) -> Result<u64,IpcError>`（上限 4→`invalid_argument`；分类错误）；`list_channels(session, query, limit, offset) -> (Vec<Mf4ChannelJson>, u32)`；`close_session(app, id)`（幂等）。
- [ ] **Step 1：** 加依赖，`cargo check -p lumelog-app` 确认 `falcon_mdf` 在 MSRV/feature 下可编译（若 mmap 需要额外 feature 才能读，记录并调整）。
- [ ] **Step 2：测试**（夹具 `falcon_mdf::Mf4Writer` 写临时 `.mf4`：时间通道 + f64/i16/文本各一）：`open_reports_counts`、`open_non_mf4_is_not_mf4`、`session_limit_four`、`channels_query_and_page`（大小写不敏感、total、越界 offset 空页）、`close_idempotent`。
- [ ] **Step 3：** FAIL → 实现 → PASS + `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。
- [ ] **Step 4：** CHANGELOG；commit `feat:mf4-sessions` + push。

---

## Task 2: `mf4_records`（时间窗 + 分页 + 值映射）

**Files:** Modify `app/src-tauri/src/mf4.rs`；Test 同文件
**Interfaces:** `records(session, channels, from_sec, to_sec, sort, limit, offset) -> Result<Mf4RowsJson,IpcError>`；`Mf4RowsJson { time_ms: Vec<Option<f64>>, columns: Vec<Mf4ColumnJson>, total_rows, total_estimated }`；`Mf4CellJson`（`Number/Text/Bool/Null`，serde 内部标签 `kind`）。
- [ ] **Step 1：测试**：`records_window_clips`（只含 `[from,to]`）、`records_paginate`（offset/limit + total）、`records_column_order_matches_selection`、`records_missing_cell_null_and_degraded`、`records_rejects_too_many_channels`（>32）、`records_rejects_limit`（>1000）、`records_no_time_channel`（`time_ms` 全 null）、`page_sort_only_within_page`。
- [ ] **Step 2：** FAIL → 实现（`stream::SignalChunks` 分块；时间通道为键；先窗口后分页；`col:` 排序仅本页重排）→ **Step 3：** PASS + clippy 净。
- [ ] **Step 4：** CHANGELOG；commit `feat:mf4-records` + push。

---

## Task 3: `mf4_summary` + CSV 导出 + 注册

**Files:** Modify `mf4.rs`（纯逻辑）、`commands.rs`（`#[tauri::command]` 入口，与 `script.rs`/`commands.rs` 分工一致）、`lib.rs`（注册 `mf4_summary/mf4_export_csv/mf4_export_cancel`）（T1/T2 同理：命令均在 `commands.rs`）
**Interfaces:** `summary(session) -> Mf4SummaryJson{stats,events,groups}`；`export_csv(app, id, channels, from_sec, to_sec, target_path, on_progress: Channel<Mf4ExportProgress>, on_done: Channel<Mf4ExportStats>) -> Result<u64,IpcError>`（prologue：会话存在、`channels` 非空、目标≠源文件；`begin_search` + `spawn_blocking` + `catch_unwind`，panic 仍发 `on_done`+`end_search`）；`Mf4ExportStats { written, scanned, cancelled, errored }`。
- [ ] **Step 1：测试**：`summary_lists_stats_and_events`；导出三测：`export_csv_matches_table`（表头/行/引号）、`export_cancel_keeps_prefix`、`export_rejects_empty_channels`。
- [ ] **Step 2：** FAIL → 实现 → PASS + 全 workspace 测试不回归 + clippy 净。
- [ ] **Step 3：** CHANGELOG；commit `feat:mf4-summary-export` + push。

---

## Task 4: 前端 `ipc.ts`

**Files:** Modify `ipc.ts`、`ipc.test.ts`
**Interfaces:** `Mf4Info/Mf4Channel/Mf4Cell/Mf4Column/Mf4Rows/Mf4Summary/Mf4ExportStats` 类型 + `mf4Open/mf4Close/mf4Channels/mf4Summary/mf4Records/mf4ExportCsv/mf4ExportCancel`。
- [ ] **Step 1：** typeof 测试（7）→ **Step 2：** FAIL → 实现 → **Step 3：** `npm test` PASS + `run build`。
- [ ] **Step 4：** CHANGELOG；commit `feat:ipc-mf4` + push。

---

## Task 5: `mf4-table.ts` + `mf4-ui.ts`（dock）

**Files:** Create `app/frontend/src/mf4-table.ts`、`mf4-ui.ts`、`mf4-table.test.ts`；Modify `styles.css`
**Interfaces:** `Mf4Table`（列式：`setData(rows: Mf4Rows, cols: Mf4Column[])`、`setPage(page,pageSize,total)` 回调 `onPage`/`onSort`、虚拟窗口渲染、缺失 `—`、`degraded` 列头 `⚠`）；`openMf4Dock(host, opts)`：概览折叠 + 通道搜索（200ms 防抖、32 列上限）+ 表格 + 时间窗/排序/导出/关闭；回调注入（不 import ipc）。
- [ ] **Step 1：测试**（jsdom）：列随选择变化、上下页回调、`from>to` 拒绝提示、缺失值渲染、`degraded` 标记、响应竞态丢弃（旧请求后到不覆盖新页）。
- [ ] **Step 2：** FAIL → 实现 + `.mf4-*` 样式 → **Step 3：** PASS + `run build`。
- [ ] **Step 4：** CHANGELOG；commit `feat:mf4-ui` + push。

---

## Task 6: `main.ts` 接线

**Files:** Modify `main.ts`
**Interfaces（行为）：** 工具栏“测量数据”按钮 + 面板 `{ id:"mf4", title:"测量数据…" }`（不入 `ACTION_DEFS`）；`openMf4()`（对话框 `.mf4` → `mf4Open` → 建视图 + 与 CAN dock 互斥）；`mf4Sessions: Map<id, Mf4View>`；会话关闭/上限/竞态处理（每次取数带自增 token，旧 token 响应丢弃）；导出走 `save()` + `mf4ExportCsv` + 进度/取消；错误经 `reportError`（`.zh`）。
- [ ] **Step 1：实现**；**Step 2：** `npm test` 不回归 + `run build`；**Step 3：** CHANGELOG；commit `feat:mf4-wiring` + push。

---

## Task 7: 四门 + 终审 + 合 main

- [ ] **Step 1：** `cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build` 全绿。
- [ ] **Step 2：GUI 手工**：真实 `.mf4`（含 CAN 通道的 Vector/CANape 文件）→ 概览、搜索、多选列、翻页、时间窗、排序两种语义、导出与表格一致；加密/非 MF4/超大文件；会话上限；与 CAN dock 互斥。
- [ ] **Step 3：** CodeReview 全分支 + 修复波 + 定向复审；`--no-ff` 合 `main`、push、`docs/claude.md` 增“MF4 测量数据查看器（增量1：表格；波形待增量2）”、删 SDD 工作区。
- [ ] **Step 4：** 合并后在 `main` 复跑 `cargo test --workspace` + `run build`。

---

## 附录 A：`falcon_mdf 0.7.1` API 实测记录（已完成调研，T1 直接照此写）
- 入口：`Mf4File::open(path)` / `open_mmap` / `from_bytes(Vec<u8>)`；`version() -> Mf4Version`（impl Display）、`channel_count()`、`data_group_count()`、`data_groups() -> &[DataGroup]`、`start_time() -> &RecordingTime`、`comment() -> &str`、`unfinalized()`、`channels()` 迭代、`find_channel(name)`、`signal(&Channel) -> Result<Signal>`、`signal_chunks(&Channel) -> Result<SignalChunks<'_>>`（逐窗口 yield `Signal`，与整读同序同量）、`signals_chunks(...)`、`aligned_signal_chunks(...)`。
- `model::Channel` 字段：`id: usize`、`index`、`channel_group_index`、`data_group_index`、`name: String`、`unit: String`、`channel_type`、`sync_type`、`data_type`、`conversion`、`bit_count`（+ 更多；判断 VLSD/master 用 `is_vlsd()`/`is_master()` 需自行核实存在性）。
- `Signal`：`name()`/`unit()`/`len()`/`channel()`/`value_at(i) -> Result<f64>`/`values() -> Result<SignalValues>`/`raw_values()`/`values_f64() -> Result<Vec<f64>>`/`iter()`/`min_max()`/`mean()`/`validity() -> Option<Vec<bool>>`/`is_valid(i)`/`valid_count()`。
- `SignalValues` 变体：`U8/U16/U32/U64/I8/I16/I32/(I64)/F32/F64/Str(Vec<String>)/Bytes/...`（枚举型，含文本）。
- 测试夹具（可直接用）：`Mf4Writer::new()` / `with_start_time_ns(i64)` → `add_group(&[f64]) -> Result<&mut WriteGroup>`（时间轴，允许乱序但不允许 NaN）/ `add_group_in(sibling, times)`；`WriteGroup::add_channel(name, unit, &[f64])`、`add_channel_with_validity(.., Option<&[bool]>)`、`add_channel_typed(name, unit, SignalValues)`、`add_channel_vlsd_str(name, unit, &[impl AsRef<str>])`、`add_channel_array(...)`；最后 `write_to_file(path) -> Result<()>`。`set_compression(bool)`/`WriteCodec` 可造压缩样本。
- 错误型：`falcon_mdf::Mf4Error`（实现 `Display`/`Error`）；`Result<T> = std::result::Result<T, Mf4Error>`。分类靠 `Display` 文本关键词（magic/encrypt/unsupported/version）降级为 `mf4_not_mf4`/`mf4_encrypted`/`mf4_unsupported`/`io`。
- 待现场核（写码时验证，不阻塞）：`Channel::is_vlsd()/is_master()` 是否存在与 `SyncType` 变体名；若不存在则用 `data_type`/`channel_type` 的 `Debug` 字串判定（`kind_of` 已按此设计，失败降级 `Unknown`）。

## 备注：接缝/决策
- `falcon_mdf` 只在 `mf4.rs` 出现；DTO 全自定义，便于换库。
- `Mf4File` 若非 `Sync`：`Arc<Mutex<..>>` 串行化同一会话取数（分页本就顺序，够用）。
- 取数窗口：`limit+1` 探测 `total_estimated` 是否还有更多（避免额外全量计数）。
- 时间用毫秒显示、秒输入（后端 `f64` 秒）；`f64` 时间戳在 JS 安全范围内（相对秒）。
- 增量2（波形）另立 spec：栅格降采样（`time_ops::Raster`）+ 自绘 canvas。
