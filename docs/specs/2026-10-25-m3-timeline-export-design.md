# M3 子项目 A · 增量2：文本日志 时序筛选 + export_lines 设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` §1（时序筛选定义）、§6.4（行过滤 + 时序退化 + 过滤结果导出）、§7.5（导出）、§9（`export_lines` 命令契约）；承接增量1 spec `docs/specs/2026-10-25-m3-filter-highlight-design.md`（`lumelog_core::filter::LineFilter`、`filter_run`/`doc_lines_at`、查询面板、子集视图）。

**Goal:** 在只读文本日志视图上，用"时间戳抽取正则 + 内置格式"或退化"行号区间"做时序筛选，并把当前"文本条件 ∧ 时序 ∧ 行号"选中的原始日志行按字节导出到文件（保留原编码与原行尾），导出为可取消的流式长任务。

**Architecture:** 扩展 `lumelog_core::filter` 的 `LineFilter` 为组合谓词（文本 AND ∧ 时间 ∧ 行号区间），时间解析用新增 `chrono`（内置格式集）。`filter_run` 复用同一谓词、只改"哪些行保留"。导出逻辑落在 **core**（`line_byte_range`/`read_window` 是 `pub(crate)`，须 core 内做原始字节拷贝）：新增 `lumelog_core::export::run(doc, matcher, writer, on_progress, cancel)`；`lumelog-app` 的 `export_lines` 命令开目标文件 + `spawn_blocking` + 流式进度/取消（镜像 `can_parse`/`filter_run`）。前端查询面板加"时序"区 + "导出"按钮。

**Tech Stack:** Rust（`lumelog-core` filter/export + `chrono`（default-features=false, "std"）+ `regex`；`lumelog-app` 命令层 + `tauri::ipc::Channel`）、TypeScript 原生 DOM。测试 `cargo test`/`vitest`，门 `tsc --noEmit`、`cargo clippy --workspace --all-targets -- -D warnings`。

## 1. 范围与非目标
**做：**
- **时序筛选**（文本日志只读视图）：用户配带命名组 `(?<ts>…)` 的抽取正则 + 选内置格式（`iso` / `epoch_s` / `epoch_ms`）+ [起,止]（同格式字符串）；勾选"保留无时间戳行"。未填抽取正则 → [起,止] 退化为 1-based **行号区间**。
- **`export_lines`**：把"文本条件 ∧ 时序 ∧ 行号"选中的行按**原始字节**（含原行尾）写入目标文件；异步 `spawn_blocking`、流式进度、可取消。
- **`filter_run` 承接时序**：请求体加可选时间/行号，谓词扩展为组合式。

**不做（后续）：** CAN 侧 `export_csv`/`export_jsonl`（信号宽表，属 CAN 增量）；自定义 strftime 格式（仅内置集）；编辑模式过滤；时区换算 UI（内置 `iso` 按 RFC3339 偏移或无偏移视为 UTC）。

## 2. 组合谓词语义
一行被保留 ⇽ 全部成立：
1. 文本（增量1）：`∀ 包含 match` ∧ `∀ 排除 ¬match`。
2. 行号区间（若给）：`lineFrom ≤ line ≤ lineTo`（1-based，单侧缺省即开区间；无抽取正则时的退化路径）。
3. 时间（若给抽取正则）：抽取 `ts` 命名组子串 → 按格式解析为毫秒 `t` → `from_ms ≤ t ≤ to_ms`（from/to 各可选）。抽取不到组/解析失败视为"无时间戳"：`keep_when_no_ts` 为真则该行仍保留（不叠加时间约束），否则丢弃。

高亮 cu16 区间只对**保留行**内的包含命中产出（逻辑不变）；时间/行号仅决定是否保留，不产额外区间。

## 3. 后端

### 3.1 `lumelog_core::filter` 扩展
- `pub enum TimeFormat { Iso, EpochSec, EpochMillis }`。
- `pub struct TimeSpec { pub extract: String /* 需含命名组 ts */, pub format: TimeFormat, pub from: Option<String>, pub to: Option<String>, pub keep_when_no_ts: bool }`。
- `LineFilter::build(specs: Vec<CondSpec>, case_sensitive: bool, time: Option<TimeSpec>, line_range: Option<(u64, u64)>) -> Result<LineFilter, CoreError>`：编译包含/排除正则；若给 `time`：编译抽取正则（缺 `ts` 命名组 → `CoreError::InvalidPattern`），把 `from`/`to` 按 `format` 解析为 `Option<f64>` 毫秒（解析失败 → `CoreError::InvalidPattern`）。`line_range` 存 `(from,to)` 1-based。
- `pub fn keep(&self, line_no: u64 /* 0-based */, text: &str) -> Option<Vec<Cu16Range>>`（签名从增量1 的 `keep(text)` 扩展为带 `line_no`）：依次判 行号区间 → 文本 AND（产区间）→ 时间。`filter_run`/`export` 调用点随之传 `line_no`。
- 时间解析（`iso`）：先 `DateTime::parse_from_rfc3339` → `timestamp_millis()`；否则依次试 `NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")` / `"%Y-%m-%d %H:%M:%S%.f"`（无偏移视为 UTC）。`epoch_s`：`s.parse::<f64>()*1000.0`；`epoch_ms`：`s.parse::<f64>()`。
- 新依赖：`crates/lumelog-core/Cargo.toml` 加 `chrono = { version = "0.4", default-features = false, features = ["std"] }`。

### 3.2 `lumelog_core::export`（新模块）
- `pub struct ExportStats { pub written: u64, pub scanned: u64, pub cancelled: bool, pub errored: bool }`。
- `pub fn run<W: std::io::Write>(doc: &mut DocFile, matcher: &LineFilter, writer: &mut W, on_progress: &mut dyn FnMut(u64), cancel: &dyn Fn() -> bool) -> ExportStats`（**不返回 `Result`**：读/写错误一律吸收进 `errored=true` 后停止，保留已写字节，`writer` 由调用方 flush）：
  - 逐行（`0..line_count`）取**解码文本**判 `matcher.keep(line, text)`（分块 `chunk_at`，与增量1 `DocLines` 同法去尾换行）；
  - 保留行：用 `doc.line_byte_range(line) -> [start,end)`（含行尾）经 `read_window` 取**原始字节**写入 `writer`（不转码，精确保编码/行尾）；
  - 每行判 `cancel`（true → `cancelled=true` 早停）；每写满一批（如 4096 行）调 `on_progress(written)`。
- `pub(crate)` 的 `line_byte_range`/`read_window` 在 core 内可达；`export` 置于 core 即为解决 app 无法调用它们的问题。

### 3.3 `lumelog-app` 命令
- **扩展 `filter_run`** 请求体：`time: Option<TimeSpecJson>`、`line_from: Option<u64>`、`line_to: Option<u64>`；`LineFilter::build` 组装（`time`/`line_range` 无效经 `from_core` → `invalid_pattern`，同步拒绝，不流空）。scan 逻辑不变，`keep` 传 `line_no`。DTO `TimeSpecJson { extractPattern, format, from, to, keepWhenNoTs }`（camelCase；`from`/`to` 为 `Option<String>`，`skip_serializing_if`）。
- **新增 `export_lines`**：`#[tauri::command] fn export_lines(state, app: AppHandle, doc_id, conditions, case_sensitive, time: Option<TimeSpecJson>, line_from, line_to, target_path: String, on_progress: Channel<ExportProgress>, on_done: Channel<ExportStats>) -> Result<u64, IpcError>`：同步 prologue 校验 doc + 组装 `LineFilter`（失败即 `fail`）+ **在此处 `File::create(target_path)`**（失败即 `fail(io)`，不进 worker）；`spawn_blocking` 内 `BufWriter` 跑 `export::run`（返 `ExportStats`）→ `writer.flush()` → **`on_done.send(stats)` 始终发** → `end_search(req_id)`；命令早返 `req_id`。`ExportProgress { written: u64 }`。
- **`export_cancel(request_id)`**：复用通用取消注册表（命中 `state.cancel` 否则 `unknown_request`）。
- `lib.rs` 注册 `export_lines`/`export_cancel`。
- `parse_frames`/`scan_filter` 里对 `LineFilter::keep` 的调用补 `line_no` 实参（增量1 代码随签名更新）。

## 4. 前端

### 4.1 查询面板扩展（`main.ts`）
- 面板新增"时序"折叠区：`[抽取正则 input]` + 格式下拉（`时间戳(ISO)` / `epoch 秒` / `epoch 毫秒`）+ `[起][止]` + 勾选`保留无时间戳行`。
- `applyQuery` 组装：抽取正则有值 → 传 `time`（`from/to` 取时间字符串输入）；为空且 [起,止] 有值 → 传 `line_from/line_to`（把输入按整数解析，非整数提示 `invalid_argument` toast 并拒绝）。都空 → 只文本条件。
- 新增"导出"按钮：`save` 对话框（默认名 `<docname>.filtered.log`）选路径 → `exportLines({...当前条件/时序/行号, targetPath, onProgress, onDone})`；进行中显示进度 + 可"取消"（`exportCancel`），完成 toast `导出 N 行`；重入守卫复用 `queryBusy`（扫描/导出共用单任务互斥）。无有效条件且无时序/行号 → 提示"当前无筛选，将导出全部 N 行"并二次确认（`ask`）。

### 4.2 `ipc.ts` / `listview.ts`
- `ipc.ts`：`filterRun` args 增 `time?: TimeSpec`、`lineFrom?`、`lineTo?`；类型 `TimeSpec{ extractPattern; format:"iso"|"epoch_s"|"epoch_ms"; from?; to?; keepWhenNoTs }`；新增 `exportLines({docId, conditions, caseSensitive, time, lineFrom, lineTo, targetPath, onProgress, onDone}): Promise<number>`（双/多 Channel，早返 id）+ `exportCancel(requestId)`；错误码复用 `invalid_pattern`/`io`/`unknown_request`/`invalid_argument`（已存在，无需新增中文表项）。
- `listview.ts`：无改动（子集视图已支持任意保留行数组）。

## 5. 错误与边界
- 抽取正则非法 / 缺 `ts` 组 / `from`、`to` 不符所选格式 / `lineFrom`/`lineTo` 非整数或 `lineFrom>lineTo` → 同步拒绝（`invalid_pattern`/`invalid_argument`），不流空、不创建导出文件。
- `target_path` 打开失败（不可写/占用/无权限）→ **prologue 同步返回 `io`**（不进 worker，避免空写）。worker 内读/写错误（磁盘满、文件轮转截断等罕见）→ `export::run` 吸收为 `errored=true` 停止、保留已写；`on_done` **始终发** `ExportStats`。前端据 `stats`：`errored` → "导出中断（已写 N 行）"，`cancelled` → "已取消，导出 N 行"，否则 → "导出 N 行"。
- 时间戳跨行非单调：逐行独立判定，不假设单调。
- 空选择导出（无有效条件且无时序/行号）：见 4.1 二次确认（否则会写出全文件副本）。

## 6. 全局约束（沿用增量1）
简体中文 UI；DTO `#[serde(rename_all="camelCase")]` + `Option` `skip_serializing_if`；`char_map` 仍不过 IPC；不整文件驻留（导出逐行原始字节流式写，`filter_run` 文本/时序判定分块）；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；无空格 commit 消息；commit 后 push；分支 `--no-ff` 合 main。

## 7. 验收
**可测：**
- `lumelog-core::filter`：`build` 带 `time`/`line_range`；`keep(line_no,text)` 组合矩阵——行号区间开/闭/单侧、时间窗命中/未命中/无 ts 行按 `keep_when_no_ts`、三格式解析（`2026-10-25T01:02:03Z`、`2026-10-25 01:02:03`、epoch s/ms）、非法 ts/正则报错。
- `lumelog-core::export::run`：夹具日志按条件+时序保留集写出**原始字节**（含 CRLF/GBK 原样、行尾保留），`on_progress` 计数、`cancel` 早停保留部分且 `cancelled=true`；保留行内容 == 源文件对应字节切片（逐字节断言）。
- `lumelog-app`：`filter_run` 接 `time`/`line_range` 后保留集正确 + 非法抽取同步 `invalid_pattern`；`export_lines` 集成测（tempfile 目标）写出行集合 + 取消；DTO camelCase。
- `ipc.ts`/`main.ts`：`TimeSpec` 类型 + `exportLines` 包装（typeof 断言镜像现有）；面板组装逻辑纯函数（抽取正则有无 → time/lineRange）可抽可测。

**GUI 手工（`tauri dev`）：**① 时间戳抽取 + `iso` + [起,止] → 只显区间内行；② 不填抽取 → 行号区间退化生效；③ "保留无时间戳行"开→无 ts 行仍在；④ 导出当前筛选 → 目标文件行内容/编码/行尾与源一致；⑤ 大文件导出中"取消" → 文件为前缀、toast 提示。

## 8. 关键取舍（已定案）
- 导出 = **原始字节拷贝**（保编码/行尾，免转码损失）；时序/行号在**同一次扫描**内判定（单一选中集）；时间解析 = **chrono 内置格式集**（自定义 strftime 后续）。
- `export` 落 **core**（`pub(crate)` 字节访问约束）；`keep` 签名加 `line_no`（增量1 调用点随之更新，行为等价）。

## 9. 分期（计划切分预告）
① core `filter` 时序/行号扩展 + `chrono` + 单测 → ② core `export::run` + 单测 → ③ `filter_run` 接 time/lineRange + DTO → ④ `export_lines`/`export_cancel` 命令 + 测 → ⑤ `ipc.ts` 扩展 + 类型 → ⑥ `main.ts` 面板时序区 + 导出流程 → ⑦ 样式/收尾 + 全量四门 + 终审 CodeReview + 合 main。
