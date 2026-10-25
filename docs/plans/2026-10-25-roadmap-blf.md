# Roadmap-2：BLF 报文源 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 直接打开 BLF（压缩/未压缩容器）→ 流式 CAN/CAN-FD 帧进 CanPanel（过滤/DBC/停止），并支持导出 CSV 宽表/JSONL。

**Architecture:** core 新增 `BlfSource: FrameSource`（`LOGG` 头 + `LOG_CONTAINER` + v1/v2 对象，跨容器 tail 续接）；app 新增 `can_parse_blf`/`can_export_blf_*`（复用 `parse_frames`/`can_cancel`/`IpcError::from_can`），`export_can` 抽成 `export_frames` 共用；前端 BLF 视图复用 CanPanel + 现有过滤器（键 `-2`）。

**Tech Stack:** Rust + `flate2 = "1"`（仅 zlib）；TypeScript。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

**Spec:** `docs/specs/2026-10-25-roadmap-blf-design.md`。分支基：`main` 当前 `b908840` → `feat/roadmap-blf`。

## Global Constraints

- 格式常量：文件头 magic `LOGG`、144B 头、`SYSTEMTIME@40`；对象 `LOBJ` + header_version 1(16B)/2(24B) 扩展；容器 10（method 0/zlib 2）；对象 1/86/100/101。
- 时间：`factor = 10µs if (flags&1)!=0 && (flags&2)==0 else 1ns`；绝对 = 文件头时间 + 相对（无基准则相对秒）。
- 上限：单对象 ≤64MiB、解压后容器 ≤256MiB；超限→`SourceFormat`（不得 OOM）。
- 跨容器分割对象必须只产出一帧；未知对象计 `skipped`（不报错）；损坏/截断→错误+停止，已产出帧保留。
- `Frame.line_number` = 帧序号（1 起）；channel 转 0 基；classic data 取 min(dlc,8)、FD100 取 min(valid,64)、FD64 按 ext_data_offset 边界。
- 导出：`export_frames` 行为与现 `export_can` 完全一致（表头/列/统计/取消），BLF 复用。
- 前端：BLF 与文档视图互斥；切标签退出 BLF；行激活在 BLF 下 toast；导出/解析共享 busy；停止复用 `can_cancel`。
- 简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；push；`--no-ff` 合 main。

## Review Focus

1. **容器/对象边界**：跨容器切分的对象只出一帧；tail 同步窗口；对象 size/padding；截断在容器/对象中途的行为。
2. **时间**：SYSTEMTIME→epoch（无效→相对）、10µs vs 1ns、1.5s 精度、既有时间过滤（ms）可用。
3. **FD 映射**：100 的 dlc_code→长度（9→12）、valid_bytes、edl/brs/esi；101 的 direction/RTR/ext_data_offset 与 64B 数据；channel 1→0。
4. **内存/健壮性**：损坏 size 不导致巨量分配；zlib 截断/超限报错；解压炸弹受限。
5. **导出等价**：`export_frames` 重构后既有 CSV/JSONL 测试全绿；BLF 导出过滤/取消/错误标记正确。
6. **BLF 视图**：无文档时工具栏/过滤/导出/停止可用；切标签恢复文档视图；坏文件不崩；重复打开重置表格。

---

## Task 1: core `lumelog-can/src/blf.rs`（读取器 + 夹具测试）

**Files:** Create `crates/lumelog-can/src/blf.rs`；Modify `crates/lumelog-can/src/lib.rs`（`pub mod blf;`）、`crates/lumelog-can/Cargo.toml`（`flate2 = "1"`）、`Cargo.lock`
**Interfaces:** `pub struct BlfSource`；`pub fn open(path: &Path) -> Result<BlfSource, CanError>`；`impl FrameSource { next_frame, stats }`；`pub fn take_error(&mut self) -> Option<CanError>`；常量按 spec §2。
- [ ] **Step 1：夹具 + 测试**（`#[cfg(test)]` 内建 writer）：`classic_frame_fields_and_time`（已知 SYSTEMTIME + rel 1.5s、ext、RTR、channel 2→1、dlc/data）；`fd100_maps_lengths_and_flags`（dlc_code 9→12、valid 12、edl/brs/esi）；`fd64_maps_64_bytes_and_direction`；`compressed_container_matches_uncompressed`（zlib）；`split_object_across_containers_emits_once`；`ten_microsecond_flag_scales`（flags=1, rel 100 → +1ms）；`unknown_objects_are_skipped`；`bad_magic_is_error`；`truncated_container_is_error`；`frame_line_numbers_increase`。
- [ ] **Step 2：** `cargo test -p lumelog-can blf::` → FAIL。
- [ ] **Step 3：实现**（`BufReader`、tail 缓冲、`next_frame` 循环、`dlc2len` 表、`systemtime_to_epoch` Hinnant、zlib `take` 上限）。
- [ ] **Step 4：** PASS + `cargo test -p lumelog-can` 不回归 + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:blf-source` + push。

---

## Task 2: 抽取 `export_frames`（行为不变）

**Files:** Modify `app/src-tauri/src/can_export.rs`；既有测试 `can_export::tests::*` 不改断言
**Interfaces:** `pub fn export_frames<W: Write>(src: &mut dyn FrameSource, dbc: Option<&Database>, filter: &CanFilterJson, writer: &mut W, on_progress: &mut dyn FnMut(u64), cancel: &dyn Fn() -> bool, wide: bool) -> CanExportStats`；`export_can` 保留原签名并改为 `build_frame_source` + `export_frames`。
- [ ] **Step 1：** 跑既有 `cargo test -p lumelog-app can_export::` 记录基线通过。
- [ ] **Step 2：实现重构**（表头/列/循环/统计原样搬移；`FrameSource` 引用）。
- [ ] **Step 3：** 同测试仍全绿（失败即回滚语义）+ clippy 净。
- [ ] **Step 4：** CHANGELOG；commit `refactor:export-frames` + push。

---

## Task 3: app 命令 `can_parse_blf` + `can_export_blf_csv/jsonl` + DTO

**Files:** Modify `app/src-tauri/src/commands.rs`、`dto.rs`（`BlfStatsJson`）、`lib.rs`（注册）；`Cargo.toml` 无新依赖
**Interfaces:**
- `can_parse_blf(app, path: String, dbc_id: Option<u64>, on_batch: Channel<Vec<FrameJson>>, on_done: Channel<BlfStatsJson>) -> Result<u64, IpcError>`：`begin_search` + `BlfSource::open` 同步校验 → `spawn_blocking`（`parse_frames`，完成后 `take_error` 写入 error）→ `end_search`。
- `can_export_blf_csv/jsonl(state, app, path, dbc_id, filter, target_path, on_progress: Channel<CanExportProgress>, on_done: Channel<CanExportStatsJson>, wide)`：`begin_search` + `open_export_target` + `BlfSource::open` → worker `export_frames`；源错误→`errored=true`。
- [ ] **Step 1：** `cargo test -p lumelog-app` 基线；新增 DTO 测试 `blf_stats_camel_case`（`error` null/有值），源错误映射由 T1 的坏 magic 测试 + `IpcError::from_can` 既有测试覆盖。
- [ ] **Step 2：** FAIL → **Step 3：实现** + 注册 → **Step 4：** PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:blf-commands` + push。

---

## Task 4: 前端 `ipc.ts` + 测试

**Files:** Modify `app/frontend/src/ipc.ts`、`ipc.test.ts`
**Interfaces:** `BlfStats{decoded:number;skipped:number;error:string|null}`；`canParseBlf({path,dbcId,onBatch,onDone}):Promise<number>`；`exportBlfCsv/exportBlfJsonl({path,dbcId,filter,targetPath,onProgress,onDone}):Promise<number>`。
- [ ] **Step 1：** `ipc.test.ts` 加 typeof 3 个新包装（复用现有 fake 模式）。
- [ ] **Step 2：** FAIL → **Step 3：实现** → **Step 4：** `npm test` PASS + build。
- [ ] **Step 5：** CHANGELOG；commit `feat:ipc-blf` + push。

---

## Task 5: `main.ts` BLF 视图（打开/停止/过滤/导出/切换）

**Files:** Modify `app/frontend/src/main.ts`
**Interfaces（行为）：**
- `const BLF_KEY = -2;`；`let blfView: { path: string; store: FrameStore } | null = null;`
- 工具栏“打开 BLF…”按钮（`open({filters:[{name:"BLF",extensions:["blf"]}]})`）→ `startBlfParse(path)`：busy 守卫；`ensurePanel`/`toggleCanDock`；`blfView={path,store:new FrameStore()}`；`store.clear`；attach + `setFilter(canFilters.get(BLF_KEY) ?? {})` + `restoreFilterInputs`；`setCanParsing(true)`；`canParseBlf`（`onBatch`→`store.push`+`canPanel.render()`；`onDone`→toast `BLF 解码 N 帧，跳过 M 个对象`，`s.error` 再 toast；`setCanParsing(false)`）。
- `onCanRunClick`：`blfView` 时 → 解析中则 `can_cancel(canReqId)`，否则 `startBlfParse(blfView.path)`。
- `switchCanView`：`blfView` 时 attach BLF store + BLF_KEY 过滤/恢复输入；否则原逻辑。
- `applyFilterFromInputs`：键 = `blfView ? BLF_KEY : activeDocId`。
- `revealSourceLine`：`blfView` 时 `toastMsg("BLF 无源文本，无法跳转到行")` 并返回。
- `tabs.onActivate(id>=0)`：`blfView = null;` 若 `canDockVisible` 则 `switchCanView()`。
- `onCanExport`：`blfView` 时用 `exportBlfCsv/Jsonl`（`path=blfView.path`，默认名 `basenameOf(path)`），其余进度/取消/文案不变。
- `setCanParsing` 文案：BLF 下 `解析中…/重新解析 BLF`。
- [ ] **Step 1：实现**；**Step 2：** `npm test` 不回归 + build tsc PASS；**Step 3：** CHANGELOG；commit `feat:blf-view` + push。

---

## Task 6: 全量四门 + 终审 + 合 main

- [ ] **Step 1：** 四门全绿（`cargo test --workspace` / clippy / `npm test` / build）。
- [ ] **Step 2：GUI 手工**：压缩与未压缩 BLF → 帧表；DBC 导入→信号；ID/时间过滤；停止保留部分；切标签回文档；BLF 导出 CSV/JSONL 与视图一致；坏文件报错不崩；未知对象统计正确。
- [ ] **Step 3：** CodeReview 全分支 + 修复波 + 定向复审；`--no-ff` 合 main、push、`docs/claude.md` 勾选 Roadmap BLF（⚠ MF4 仍暂缓）、删 SDD 工作区。
- [ ] **Step 4：** 合并后 `main` 复跑 `cargo test --workspace` + `run build`。

---

## 备注：接缝/决策
- `BlfSource` 是 `FrameSource`：`parse_frames`/`export_frames` 零改动复用；错误经 `take_error` 单独上报（`stats` 只含 decoded/skipped）。
- zlib 解压用 `read::ZlibDecoder(...).take(MAX).read_to_end`；读满上限且未 EOF→错误。
- 顶层非容器对象只计数跳过；内层未知版本跳过；内层损坏/顶层损坏 → `SourceFormat` 停止。
- BLF 视图过滤器存 `canFilters` 的 `BLF_KEY`，与文档过滤互不影响；切标签丢弃 BLF 视图。
- 导出默认文件名用 BLF 基名；`dbc_id` 仍用全局 `lastDbcId`。
