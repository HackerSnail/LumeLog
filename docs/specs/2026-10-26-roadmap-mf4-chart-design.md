# Roadmap-4 增量2：MF4 自绘 canvas 波形设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` Roadmap（MF4）；承接增量1 `docs/specs/2026-10-26-roadmap-mf4-viewer-design.md` 与 `docs/reviews/2026-10-26-mf4-review-findings.md`。本增量只加「波形」视图，不改增量1 的表格/导出契约。

**Goal:** 在 MF4 dock 内用自绘 canvas 显示选中通道的**时间-数值包络**：滚轮以光标为中心缩放、拖拽平移、双击复位、hover 十字准星显示各序列真值；数据由后端按窗口做 **min/max 降采样**，不把整条信号送到前端。

**Architecture:** 后端新增 `mf4_series`（`mf4.rs`，与 `records` 同一「同数据组 + 时间主通道」规则），流式扫描窗口并按桶输出 `min/max`；前端新增 `mf4-chart.ts`（纯函数 + 双层 canvas：静态层 + hover 层），由 `mf4-ui.ts` 以 tab 形式挂到现有 dock，`main.ts` 注入 IPC。

**Tech Stack:** Rust `falcon_mdf 0.7`（`aligned_signal_chunks` 优先，退化 `signal_chunks`）；TypeScript 无新依赖（canvas 2D、`ResizeObserver`、`Pointer Events`）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

## 1. 范围与非目标
- 做：dock 内「表格 / 波形」tab；后端 min/max 桶化；canvas 网格/坐标轴/时间刻度；多序列包络（≤8）；缩放（wheel，光标为锚点）、平移（pointer 拖拽）、复位（dblclick）；hover 十字线 + tooltip（时间 + 各序列原值）；窗口变化自动重取；DPR + ResizeObserver；「应用到表格」把波形窗口写回表格时间窗。
- 不做：表格↔图双向联动/选行高亮、多 Y 轴、游标差值测量、FFT/统计、图片导出、插值重采样（`Raster`）、序列样式自定义、>8 序列。

## 2. 后端命令 `mf4_series`
```
mf4_series(id, channels: [u32], fromSec: Option<f64>, toSec: Option<f64>, buckets: u32)
  -> Mf4SeriesJson
Mf4SeriesJson {
  hasTimeAxis: bool,
  fromSec: f64, toSec: f64,      // 实际使用的窗口（入参为 None 时取该组时间轴的起/止）
  bucketSec: f64,                 // 桶宽 = 窗口/buckets
  times: Vec<Option<f64>>,        // 每桶中点（秒），空桶 null
  series: Vec<Mf4SeriesEntryJson>,
}
Mf4SeriesEntryJson {
  index: u32, name: String, unit: String, degraded: bool,
  min: Vec<Option<f64>>, max: Vec<Option<f64>>,
  first: Vec<Option<f64>>, last: Vec<Option<f64>>,
}
```
- 校验：`channels` 非空且 ≤8；`buckets` 1–4000；`from ≤ to`；与 `records` 相同的跨数据组拒绝（`invalid_argument`）；无时间通道 → `hasTimeAxis=false`、`series` 各数组全 `null`（前端禁用波形）。
- 取数：优先 `aligned_signal_chunks` 一次对齐组内序列；若签名/行为不满足（写码时验证），退化为逐通道 `signal_chunks` + 组主通道时间轴按**绝对样本下标**对齐，并在代码注释与 CHANGELOG 记录该假定（同组等采样率成立；混采样在后续增量用 `Raster` 解决）。
- 桶化：桶号 `bucket = floor((t - winFrom) / bucketSec)` 夹到 `[0, buckets-1]`；每桶维护 `min/max/first/last` 与是否见过样本；`first`/`last` 取桶内首/末样本值（供 hover 真值与可选细线）。
- 单遍流式、O(窗口样本)，内存 O(buckets × series)；超过 1e7 样本的窗口允许慢（一次全扫），但绝不整信号物化。
- 非数值通道（文本/`Bytes`/`Complex`）→ `invalid_argument`（前端也先挡）；读取失败的序列 `degraded=true`、全 `null`。

## 3. 前端 `mf4-chart.ts`
- 导出的纯函数（vitest 直接测）：`clampWindow(win, full)`、`zoomAt(win, factor, anchorT, full)`、`panBy(win, dtSec, full)`、`bucketIndex(t, win, buckets)`、`nearestBucket(times, tSec)`、`yOf(v, min, max, top, height)`、`xOf(t, win, left, width)`。
- `Mf4Chart` 类：`setData(Mf4SeriesJson)`、`setWindow(win)`、`setHover(t | null)`、`setBusy(bool)`、`onWindowChange(cb)`、`destroy()`。
  - 双层 canvas（静态 `base` + 透明 `overlay`）；`ResizeObserver` + `devicePixelRatio` 同步尺寸；变化用 `requestAnimationFrame` 合并重绘。
  - 静态层：背景/网格（4×5 等分）、时间刻度（自动选 1/2/5×10ⁿ 步长，格式化 `s/ms`）、每序列包络：每桶一条 `min→max` 竖线 + 桶间 `last→next first` 细连线（`globalAlpha` 降级，8 序列也不糊）；配色固定 10 色循环；选中（hover 最近序列或点击图例）加粗。
  - overlay 层：十字准星（垂直 + 当前序列水平）、tooltip（HTML `div`，绝对定位，含时间与每条序列名/单位/值；`first` 缺失显示 `—`）。
- 交互：wheel（`{passive:false}`，`factor=1.25^(−deltaY/100)`，锚点 = 光标时间，夹紧到 `[full.start, full.end]` 且最小窗宽 `max(fullSpan/1e6, 1e-6)`）；pointerdown/move/up + `setPointerCapture` 拖拽平移（像素→秒，夹紧）；dblclick 复位到全范围。窗口变化回调节流 60 ms。
- 渲染限制：`buckets = clamp(floor(cssWidth × dpr × 2), 200, 4000)`（缩放后同上）；绘制时跳过全 `null` 的桶。

## 4. dock 集成（`mf4-ui.ts` / `main.ts`）
- `Mf4View` 增加 tab：`表格`（增量1 原样）/`波形`。切到波形时用当前选中通道与当前时间窗立即取数；通道选择变化（数值通道才可入图）与窗口变化都会触发 `onSeriesQuery`。
- 波形工具栏：显示当前窗口 `起 – 止 s`、「应用到表格」（把窗口写入表格的起/止输入并触发查询）、「复位」、图例（点击高亮/隐藏序列，仅前端）。
- `main.ts`：新增 `mf4Series` IPC 注入、`seriesToken` 竞态丢弃、波形 busy/错误进状态区；导出进行中仍可绘制（后端锁已分离）；`closeMf4` 取消进行中的 series 请求（token 失效即可）。
- 缩放/平移只改**波形**窗口；表格时间窗保持不变，直到用户点「应用到表格」。

## 5. 边界与错误
- 无时间轴：波形 tab 禁用（提示「该文件无时间轴，无法绘制波形」）。
- 窗口内无样本：画「该区间无样本」，不复位窗口。
- 全 `degraded` 序列：显示表头 `⚠` 与「无可绘制数据」。
- 通道数 >8：第 9 个不可勾选/提示；文本通道在波形模式下显示为禁用项。
- 缩放穿越全范围：夹紧到全范围；零宽窗口被 `clampWindow` 拒绝。
- DPR 变化（拖到另一屏）/窗口缩放/主题切换：重绘（主题色从 CSS 变量读取一次/重绘时读取）。

## 6. 验收
**Rust**（`mf4.rs` 测试，沿用 `Mf4Writer` 夹具）：
- `series_bucket_count_and_limits`（buckets 0/4001 → `invalid_argument`；正常返回桶数 ≤ buckets）
- `series_min_max_keeps_spikes`（构造 1 个远高尖峰，验证落在某桶 `max`）
- `series_first_last_and_empty_buckets`（空桶 `None`；`first/last` 与样本一致）
- `series_rejects_cross_group_and_non_numeric`
- `series_without_time_axis`（`hasTimeAxis=false`，数组全空/全 `null`）
**前端**：`mf4-chart.test.ts`（纯函数：`zoomAt` 锚点时间不变、`panBy` 夹紧、`nearestBucket` 边界、`yOf` 单调、`clampWindow` 最小宽）；`mf4-ui` 新 tab（jsdom：切 tab 触发 `onSeriesQuery`、`应用到表格` 写回输入、无时间轴时禁用）+ 不回归；`npm test` + `run build`。
**手工**：真实多组 MF4，8 序列叠画；wheel/拖拽/dblclick 手感；hover 与表格数值一致；1M+ 样本缩放仍流畅；窗口缩放/主题切换/DPR 变化；导出进行中切到波形。

## 7. 全局约束（沿用）
简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`；分支基 `main` 当前 HEAD（`56dc966`）→ `feat/roadmap-mf4-chart`。

## 8. 分期（计划预告）
① `mf4_series` 桶化 + 测试 → ② `mf4_series` 命令 + 注册 + `ipc.ts` 包装/测试 → ③ `mf4-chart.ts` 纯函数 + 测试 → ④ `Mf4Chart` 渲染/交互 + jsdom 测 → ⑤ `mf4-ui.ts` tab + `main.ts` 注入 + 测试 → ⑥ 四门 + 终审 + 修复波 + 合 `main`。

## 9. 关键取舍
- **包络而非抽样**：日志/故障分析必须保尖峰；2×像素宽的数据量可控（4000 桶 × 8 序列 × 4 数组 ≈ 可接受）。
- **归一化 + hover 真值**：不同量级同屏可读；真值放 tooltip，不受 Y 轴拥挤影响。
- **双层 canvas**：hover 60 fps 重绘不牵动静态层。
- **窗口独立 + 手动同步**：缩放不惊动表格分页；「应用到表格」是显式动作，避免每帧重取表格数据。
- **不引入图表库**（延续增量1），图表只依赖 `mf4_series` 列式 DTO。
