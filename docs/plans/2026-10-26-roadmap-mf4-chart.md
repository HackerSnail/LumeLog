# Roadmap-4 增量2：MF4 自绘 canvas 波形 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 MF4 dock 内加「波形」tab：后端 min/max 桶化 + 前端两层 canvas（滚轮缩放/拖拽平移/双击复位/hover 真值）。

**Architecture:** `mf4.rs` 新增 `series()`（同组+时间主通道、单遍流式桶化）；`commands.rs` 加 `mf4_series`；`ipc.ts` 类型化包装；新 `mf4-chart.ts`（纯函数 + `Mf4Chart` 双层 canvas）；`mf4-ui.ts` 加 tab；`main.ts` 注入 + `seriesToken`。

**Tech Stack:** `falcon_mdf 0.7`（`aligned_signal_chunks` 优先）；TS 无新依赖（canvas 2D / Pointer Events / ResizeObserver）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

**Spec:** `docs/specs/2026-10-26-roadmap-mf4-chart-design.md`。分支基：`main`（`a52503d`）→ `feat/roadmap-mf4-chart`。

## Global Constraints

- `buckets` 1–4000；`channels` 1–8；`from ≤ to`；跨数据组 / 非数值 → `invalid_argument`。
- 与 `records` 相同的主通道规则：同一 `data_group_index` 且 `is_time_channel()`；无时间通道 → `hasTimeAxis=false` + 全 `null`。
- `times` 为桶中点（秒）；空桶 `null`；`min/max/first/last` 均 `Option<f64>`（camelCase、untagged 不需要）。
- 单遍流式，禁止整信号物化；内存 O(buckets × series)。
- 缩放最小窗宽 `Math.max(fullSpan / 1e6, 1e-6)`，夹紧到全范围。
- 简体中文 UI；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. **对齐**：`aligned_signal_chunks` 是否可用、与 `signal_chunks` 的绝对下标是否一致；混采样率通道是否错位（spec 明确退化假定）。
2. **桶边界**：`t==winTo` 落最后一桶不越界；`first/last` 取桶内首末；尖峰进 `max`。
3. **窗口数学**：`zoomAt` 锚点不动、`panBy` 两端夹紧、最小窗宽；`nearestBucket` 对 `null` 桶的处理。
4. **渲染**：DPR/ResizeObserver 后 canvas 尺寸与 `ctx.scale` 只设一次；双层 canvas 的 overlay 清理；空数据/全 degraded。
5. **竞态**：缩放节流 + token 丢弃旧响应；切 tab/关闭会话时不再更新。
6. **集成**：数值通道限制、>8 拒绝、「应用到表格」只写窗口不改列选择。

---

## Task 1: `mf4_series` 桶化（`mf4.rs`）

**Files:** Modify `app/src-tauri/src/mf4.rs`；Test 同文件
**Interfaces:** `pub(crate) fn series(sessions:&Mf4Sessions,id:u64,channels:&[u32],from_sec:Option<f64>,to_sec:Option<f64>,buckets:u32) -> Result<Mf4SeriesJson,IpcError>`；`Mf4SeriesJson{has_time_axis,from_sec,to_sec,bucket_sec,times:Vec<Option<f64>>,series:Vec<Mf4SeriesEntryJson>}`；`Mf4SeriesEntryJson{index,name,unit,degraded,min,max,first,last}`（后四者 `Vec<Option<f64>>`，长度 = 桶数）；常量 `MAX_SERIES=8`、`MAX_BUCKETS=4000`。
- [ ] **Step 1：探针**：读 `falcon_mdf` 的 `aligned_signal_chunks` 签名（`file.rs`/`stream.rs`）。可用则一次对齐取多通道；不可用则逐通道 `signal_chunks` + 组主通道按绝对下标对齐（注释记录假定）。
- [ ] **Step 2：测试**：`series_bucket_limits`（0/4001 → invalid_argument）、`series_spike_kept_in_max`（构造 0,1,2,3s + 尖峰 999）、`series_first_last`（桶内首末与样本一致）、`series_empty_buckets_are_null`（窗口宽 > 样本覆盖区间）、`series_rejects_cross_group_and_text`、`series_without_time_axis`。
- [ ] **Step 3：** FAIL → 实现（`from/to` 为 None 时用该组时间轴首末；`bucket = ((t-from)/bucket_sec).floor().clamp(0, buckets-1)`；每桶累积）→ PASS + `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。
- [ ] **Step 4：** CHANGELOG；commit `feat:mf4-series` + push。

---

## Task 2: 命令 + 注册 + `ipc.ts`

**Files:** Modify `app/src-tauri/src/commands.rs`、`lib.rs`、`app/frontend/src/ipc.ts`、`ipc.test.ts`
**Interfaces:** `#[tauri::command] mf4_series(state, id, channels, from_sec, to_sec, buckets) -> Result<Mf4SeriesJson,IpcError>`；`mf4Series(args:{id,channels,fromSec?,toSec?,buckets}) -> Promise<Mf4Series>`；类型 `Mf4Series/Mf4SeriesEntry`。
- [ ] **Step 1：** typeof 测试（+1）+ 成功路径类型编译。
- [ ] **Step 2：** FAIL → 实现 → PASS；`cargo test --workspace` 不回归 + clippy 净 + `npm build`。
- [ ] **Step 3：** CHANGELOG；commit `feat:ipc-mf4-series` + push。

---

## Task 3: `mf4-chart.ts` 纯函数

**Files:** Create `app/frontend/src/mf4-chart.ts`、`mf4-chart.test.ts`
**Interfaces:** `type Win = { start:number; end:number }`；导出 `clampWindow(win, full): Win`、`zoomAt(win, factor, anchorT, full): Win`、`panBy(win, dtSec, full): Win`、`bucketIndex(t, win, buckets): number`、`nearestBucket(times:(number|null)[], t): number`、`xOf(t, win, left, width): number`、`yOf(v, min, max, top, height): number`。
- [ ] **Step 1：测试**：`zoom_keeps_anchor`（缩放前后 `xOf(anchor)` 相对位置不变 ±1e-9）、`zoom_clamps_to_full_and_min_width`、`pan_clamps_both_ends`、`nearest_bucket_ignores_null_and_edges`、`y_monotonic_and_flat_series_safe`（`min==max` 不除零）、`bucket_index_edges`（`t==end` → 最后一桶）。
- [ ] **Step 2：** FAIL → 实现（纯函数、无 DOM）→ PASS。
- [ ] **Step 3：** CHANGELOG；commit `feat:mf4-chart-math` + push。

---

## Task 4: `Mf4Chart` 渲染与交互

**Files:** Modify `mf4-chart.ts`；Create/Modify `mf4-chart.dom.test.ts`
**Interfaces:** `new Mf4Chart(host: HTMLElement)`；`setData(data: Mf4Series)`、`setWindow(win: Win)`、`setBusy(b)`、`onWindowChange(cb:(win:Win)=>void)`、`destroy()`；内部两层 canvas、`ResizeObserver`、wheel/pointer/dblclick、`rAF` 合并、10 色板。
- [ ] **Step 1：jsdom 测**：挂载后存在两层 canvas；`setData` 后基 canvas 有宽高；wheel 后 `onWindowChange` 只触发一次（节流）；dblclick 回调 `{start:full.start,end:full.end}`；`destroy` 后事件不再触发（派发 wheel 无回调）。
- [ ] **Step 2：** FAIL → 实现（`passive:false`；`getContext` 缺失时安全降级：不抛错、仅回调）→ PASS + `run build`。
- [ ] **Step 3：** CHANGELOG；commit `feat:mf4-chart-canvas` + push。

---

## Task 5: dock tab + `main.ts` 注入

**Files:** Modify `mf4-ui.ts`、`mf4-ui.test.ts`、`main.ts`、`ipc.ts`（如需类型导入）
**Interfaces（行为）：** dock 顶部「表格 / 波形」tab；波形模式：数值通道才可勾（文本禁用）、>8 提示、「应用到表格」写回起/止并触发 `query()`、「复位」回全范围；`onSeriesQuery({channels,fromSec,toSec,buckets})`；`setSeries(data)`；无时间轴 tab 禁用。
- [ ] **Step 1：jsdom 测**：切 tab → `onSeriesQuery` 一次（含当前选择/窗口）；`应用到表格` 后 `onQuery` 的 `fromSec/toSec` 等于波形窗口；无时间轴时 tab `disabled`；文本通道 checkbox `disabled`。
- [ ] **Step 2：** FAIL → 实现 → PASS。
- [ ] **Step 3：`main.ts`**：`mf4Series` 注入 + `seriesToken` 丢弃旧响应 + busy/错误进状态区 + `closeMf4` 使 token 失效；`run build`。
- [ ] **Step 4：** CHANGELOG；commit `feat:mf4-chart-ui` + push。

---

## Task 6: 四门 + 终审 + 合 main

- [ ] **Step 1：** `cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build` 全绿。
- [ ] **Step 2：GUI 手工**：真实多组 MF4 8 序列叠画；wheel/拖拽/dblclick；hover 与表格数值一致；1M+ 样本缩放流畅；窗口缩放/主题/DPR；导出中切波形。
- [ ] **Step 3：** CodeReview 全分支 + 修复波 + 定向复审；`--no-ff` 合 `main`、push、`docs/claude.md` 勾选「MF4 波形（增量2）」，删分支。
- [ ] **Step 4：** 合并后在 `main` 复跑四门。

---

## 备注
- 纯函数与 DOM 分离：数学全部在 Task 3 落地，Task 4 只负责像素/事件。
- jsdom 无真实像素：像素级正确性靠手工验，测试只断言尺寸/回调/节流/降级路径。
- 若 `aligned_signal_chunks` 不可用：T1 的注释和 CHANGELOG 必须写明「同组等采样率」假定，并在 spec 的「不做」里保留混采样说明。
