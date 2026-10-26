# Roadmap-4 增量1：MF4 测量数据查看器（元数据 + 通道 + 记录表 + 导出）设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` Roadmap（MF4）。承接已交付：`export_frames`/`export_can` 的流式导出模式、`begin_search/cancel` 取消注册表、CanPanel 的 `@tanstack/table-core` + 自写窗口化渲染、命令面板/工具栏与 dock 模式。格式解析采用 `falcon_mdf`（不自研）。

**Goal:** 直接打开 `.mf4`（ASAM MDF4）测量文件，查看文件元数据、搜索并多选通道、按时间窗**分页浏览记录表格**，并把选定通道在选定时间窗内**流式导出为 CSV**（可取消）。

**Architecture:** 新模块 `app/src-tauri/src/mf4.rs` 是唯一接触 `falcon_mdf` 的边界，其余只看到自有 DTO。`state.rs` 新增 `Mf4Sessions`（`id →` 已打开文件 + 通道元数据缓存）。MF4 是独立「会话 + 底部 dock」，**不走** 文本文档/行索引链路。前端 `mf4-ui.ts`（通道面板）+ `mf4-table.ts`（列式虚拟表格），全部数据按窗口/分页从后端拉取。

**Tech Stack:** Rust：`falcon_mdf = { version = "0.7", default-features = false, features = ["mmap"] }`；workspace `rust-version` 1.87 → **1.89**（该库要求）。TypeScript 无新依赖（复用 table-core）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

## 1. 范围与非目标
- 做：`mf4_open/close/channels/records/summary/export_csv`；元数据概览；通道搜索+多选（上限 32 列）；记录表格（500 行/页、时间或列排序、缺失值 `—`）；CSV 导出（列式、流式、可取消）。
- 不做（后续增量/项）：**波形图（增量2）**、通道运算/单位切换、CAN 语义解码（`candb`）、MF4 写入、MF3/MF2、多文件对比、图表导出、真实样式的 3D/直方图。

## 2. 依赖边界（`mf4.rs`）
- 打开：`Mf4File::open(path)`（`OpenOptions` 走 mmap；关闭时随 `Arc` drop）。失败分类映射自有错误码：`mf4_not_mf4`（magic/结构）、`mf4_encrypted`（加密块）、`mf4_unsupported`（版本/块特性不支持）、`io`。
- 会话注册表 `Mf4Sessions`（`state.rs`，与 `AppDocs` 同风格）：
  `struct Mf4Session { file: Arc<Mutex<Mf4File>>, channels: Vec<Mf4ChannelMeta> }`，
  `Mf4ChannelMeta { index:u32, name:String, unit:String, kind:Mf4ValueKind, sync:bool, group:u32, source:String }` —— 打开时一次性解析（falcon_mdf 结构是即时解析、样本惰性），后续列表/取列都基于缓存，不重复遍历块。
- 记录取数：优先 `stream::SignalChunks`（有界分块）按 `[fromSec,toSec]` + `[offset,offset+limit)` 取窗口；无时间通道时用记录序号（`timeMs` 为 null、UI 标「无时间轴」）。变长/复杂类型取不到值时该单元为 `null`。

## 3. 后端命令与 DTO（camelCase）
- `mf4_open(app, path) -> Mf4InfoJson { id, version, start_iso: Option<String>, duration_sec: Option<f64>, channel_count: u32, data_groups: u32, events: u32, attachments: u32, app: String, author: String }`
- `mf4_close(app, id) -> ()`（幂等）
- `mf4_channels(app, id, query: Option<String>, limit: u32, offset: u32) -> Mf4ChannelsPage { items: [Mf4ChannelJson], total: u32 }`（`Mf4ChannelJson { index, name, unit, kind, sync, group, source }`；大小写不敏感子串匹配，按名称稳定排序）
- `mf4_summary(app, id) -> Mf4SummaryJson { stats: [Mf4KvJson], events: [Mf4EventJson], groups: [Mf4GroupJson] }`（`FileStatistics`/`Metadata` 键值 + 事件名/时间 + 组名与通道数）
- `mf4_records(app, id, channels: Vec<u32>, from_sec: Option<f64>, to_sec: Option<f64>, sort: String, limit: u32, offset: u32) -> Mf4RowsJson { time_ms: Vec<Option<f64>>, columns: Vec<Mf4ColumnJson>, total_rows: u64, total_estimated: bool }`，`Mf4ColumnJson { index, name, unit, values: Vec<Mf4CellJson>, degraded: bool }`，`Mf4CellJson = Number(f64)|Text(String)|Bool(bool)|Null`；`sort ∈ {"timeAsc","timeDesc", "col:<index>Asc", "col:<index>Desc"}`；`limit ≤ 1000`（超出→`invalid_argument`），`channels.len() ≤ 32`。
- `mf4_export_csv(app, id, channels, from_sec, to_sec, target_path, on_progress: Channel<Mf4ExportProgress>, on_done: Channel<Mf4ExportStats>) -> reqId`（复用 `begin_search`/`end_search`；`Mf4ExportStats { written, scanned, cancelled, errored }`）+ `mf4_export_cancel(reqId)`。
- `lib.rs` 注册 7 命令。**排序语义**：`timeAsc|timeDesc` 是跨页的全局排序（以时间通道为键，offset/limit 在排序后的序列上分页）；`col:<index>Asc|Desc` **仅对当前页的行重排**（不跨页），UI 标“本页排序”，避开需物化整个时间窗的代价。

## 4. 前端
- 工具栏“测量数据”按钮 + 命令面板 `{ id:"mf4", title:"测量数据…", run: openMf4 }`（不进 `ACTION_DEFS`，与 `keymap`/`script` 同处理）。`openMf4()` = 文件对话框 `.mf4` → `mf4Open` → 打开 MF4 dock 并新建视图状态；会话存 `mf4Sessions: Map<id, Mf4View>`，与文档标签无关。
- dock 三个区：
  1) **概览**（可折叠）：`Mf4InfoJson` + `mf4_summary` 键值/事件。
  2) **通道**（左栏）：搜索框（200ms 防抖，走后端分页）+ 复选列表 + `已选 N/32`；勾选变化触发取数。
  3) **表格**（主区）：`Mf4Table` 虚拟表格（列 = 时间 + 选定通道；行 = `records` 当前页），底部 `上一页/下一页/页码`、时间窗输入（起/止 秒）、排序下拉、每页 500（可选 100/500/1000）；单元格缺失显示 `—`，`degraded` 列头显示 `⚠`（tooltip 说明）。
- 导出：`save()` 选目标 `.csv`（默认 `<mf4 基名>.csv`）→ `mf4ExportCsv`（当前时间窗 + 当前列选择），状态区显示进度、可取消；toast 汇总。
- 与 CAN dock 互斥（同时只显示一个）；关闭最后一个 MF4 会话时 `mf4Close` + dock 收起。快捷键：MF4 面板内输入框不劫持全局快捷键（沿用 `tabindex`/`keydown` 停止传播的既有做法）。

## 5. 错误 / 边界
- 非 MF4/加密/不支持 → 明确 `.zh` toast，且不产生半开会话（prologue 失败即回收）。
- 时间窗无数据 → 空表格 + “该区间无样本”；`from > to` → 前端拒绝并提示。
- 大会话/大文件：所有取数按窗口与页；`total_rows` 优先取统计，缺失时以“已扫到 + 估算”标记（UI 显示 `≈`）。
- 会话上限 4：超出提示“先关闭一个测量文件”。
- 导出中途取消：保留已写前缀，状态显示“已取消”；写盘失败 → `errored`。
- 列选择为空时导出/取数一律拒绝（提示先勾选通道）。

## 6. 验收
**Rust 单测**（夹具用 `falcon_mdf::Mf4Writer` 在测试里生成临时 `.mf4`，含时间通道 + 2–3 个不同类型通道；若写入器不支持所需特性，退化为“必需样本文件 + 明确注明”）：
- 打开/关闭/会话上限；非 MF4 → `mf4_not_mf4`。
- `mf4_channels` 搜索/分页/total；`kind`/`sync` 映射。
- `mf4_records`：时间窗裁剪、分页 offset/limit、列顺序与 `channels` 一致、缺失单元 `null`、`limit>1000` 拒绝。
- `mf4_export_csv`：行数/表头/引号、取消返回 `cancelled` 且前缀可读。
**前端**：`ipc` 包装 typeof；`mf4-table.test.ts`（jsdom：表头随列变化、页码/上下页回调、缺失值渲染 `—`、degraded `⚠`）；`npm test` 不回归 + `run build`。
**GUI 手工**：真实 Vector/CANape `.mf4` → 元数据、通道搜索、多选列、翻页、时间窗、导出与表格一致；加密与超大文件；无时间通道文件的“无时间轴”提示。

## 7. 全局约束（沿用）
简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`；分支基 `main` 当前 HEAD → `feat/roadmap-mf4-viewer`。

## 8. 分期（计划切分预告）
① MSRV 1.89 + 依赖 + `mf4.rs` 会话/元数据 + `mf4_open/close/channels` + 夹具测 → ② `mf4_records`（窗口/分页/单元格映射）+ 测 → ③ `mf4_summary` + `mf4_export_csv`/`cancel` + 注册 + 测 → ④ 前端 `ipc.ts` → ⑤ `Mf4Table` + `mf4-ui.ts` dock + jsdom 测 → ⑥ `main.ts` 接线（工具栏/面板/互斥/导出） → ⑦ 四门 + 终审 + 合 main。**波形图为增量2（另立 spec/计划）。**

## 9. 关键取舍（已定）
- **用 `falcon_mdf` 而非自研**：MF4 块表/记录布局/转换/压缩体量远超 BLF；只开 `mmap` feature 求可复现，必要时再开 `parallel`。
- **MF4 是会话 + dock，不是文本文档**：避免二进制进入行索引/编码链路。
- **列式 + 后端分页**：前端只拿当前页；波形（增量2）再引入栅格降采样。
- 首版排序：时间序（跳页生效）+ 列序（仅本页）
- 导出复用既有流式/取消骨架，不引入新依赖。
