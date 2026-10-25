# M3 子项目 A · 增量3：CAN 报文 export_csv / export_jsonl 设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` §7.5（导出：信号 CSV 帧×信号宽表 物理值 / JSONL）、§9（`export_csv`/`export_jsonl` 命令：筛选描述 + 目标路径 → 长任务 channel）、§7.4（CanPanel 视图过滤）；承接增量2/增量1（`export_lines`/`export::run`、`filter_run`/`LineFilter`、`can_parse`/`FrameSource`/`frame_json`、`canfilter.ts`）。

**Goal:** 从后端把一份 CAN 日志按当前视图过滤**重新解码**并**流式**导出为**宽表 CSV**（meta 列 + 每个 DBC 信号一列、物理值）与 **JSONL**（每帧一个对象），带进度与取消。

**Architecture:** 复用 `can_parse` 的解码链（`DocLines` → `Box<dyn FrameSource>`（candump/asc/regex）→ `decode_signals`）：把"按 `source_kind`/`regex` 建源"从 `can_parse` 抽为共享 `build_frame_source`。新增后端 `CanExportFilter` + 纯函数 `frame_matches(frame, decoded_signals, &filter)`（**逐条对齐前端 `canfilter.ts`** 的 ID/消息名/信号条件/时间窗）。两命令 `export_csv`/`export_jsonl` 走 `export_lines` 同款异步壳（同步 prologue 校验/建目标文件 → `spawn_blocking` 持 doc 锁逐帧解码+过滤+写、`on_progress` 节流、`on_done`、`end_search`、早返 `req_id`）。CSV 列集由已导入 `Database.messages` 派生（确定性、可流式）；JSONL 复用现有 `frame_json`。

**Tech Stack:** Rust（`lumelog-app` 命令 + `lumelog-can`（`FrameSource`/`decode_signals`/`Database`/`frame`）+ `serde_json` + `csv` 或手写引号转义）、TypeScript 原生 DOM。门：`cargo test`、`cargo clippy --workspace --all-targets -- -D warnings`、`npm test`、`run build`（tsc）。

## 1. 范围与非目标
- 做：后端 `export_csv`（宽表、物理值）、`export_jsonl`（帧对象行）；与 CanPanel 视图过滤一致的服务端 `frame_matches`；`build_frame_source` 抽取共用；`export_cancel` + 进度；前端 CanPanel 工具条"导出 CSV…/导出 JSONL…"（取当前 `source_kind`/`regex`/`lastDbcId`/`CanFilter`）。
- 不做：原始日志行 `export_lines`（增量2 已交付）、前端序列化导出、二进制格式。

## 2. 数据模型 / 列派生（CSV 宽表）
- **meta 列（固定序）**：`lineNumber`(1-based)、`timestampMs`(空则空)、`channel`、`canIdHex`(`0x..`)、`msgName`(无则空)、`dlc`、`dataHex`。
- **信号列**：遍历 `Database.messages`（顺序）× 每 `Message.signals`（顺序），列名**限定为 `消息名.信号名`**。帧按 `can_id` 匹配 `Message`（同 `frame_json` 的 id 匹配），命中则 `decode_signals` 得各 `SignalValue.physical`，写入对应 `msgName.signalName` 列；其余信号列留空。
- 列集**仅来自 DBC**（导入时已定，故流式无需缓冲全帧）。同名 `消息名.信号名` 去重：首个胜出、后续追加 `#n`（罕见；记决策）。无 `dbc_id`：无信号列，CSV 退化为仅 meta 列。
- 物理值写入用**稳定 ASCII 浮点格式**（`{:.6}` 后裁尾零，不用 locale/科学计数于常规值），保证跨环境一致、Excel 可读；`enum_text`/`raw` 不进 CSV（走 JSONL 结构化 `signals`）。

## 3. JSONL
每帧一行 `serde_json::to_string(&FrameJson)`（复用 `frame_json(&Frame, db)`），字段与前端 `can_parse` 完全一致（含 `signals[]` 的 `name/raw/physical/enumText/unit`），`\n` 结尾。天然流式、无列集问题。

## 4. 过滤 `frame_matches`（Rust 端口，须与 `canfilter.ts` 一致）
作用在 `FrameJson` 上（与前端 `canfilter.ts` 的 `applyCanFilter(frames: FrameJson[], f: CanFilter)` **同构**，逐字端口）：
`pub struct CanFilterJson { id_from:Option<u32>, id_to:Option<u32>, msg_name:Option<String>, signal_name:Option<String>, signal_op:Option<SignalOp>, signal_value:Option<f64>, time_from_ms:Option<f64>, time_to_ms:Option<f64> }`（serde camelCase：`idFrom/idTo/msgName/signalName/signalOp/signalValue/timeFromMs/timeToMs`，均 `skip_serializing_if`）；`SignalOp{Gt,Lt,Ge,Le,Eq}`。
`fn frame_matches(fj:&FrameJson, f:&CanFilterJson) -> bool`：
- ID：`id_from`/`id_to` 对 `fj.canId` 闭区间，`id_from>id_to` → 不匹配。
- 消息名：设了则需 `fj.msgName == Some(msg_name)`（无 msgName 的帧不匹配）。
- 信号条件：`signal_name`+`op`+`value` 三者齐才生效；取 `fj.signals` 中同名 `physical` 比较；无该信号 → 不匹配；`Eq` 用**精确 `f64` 相等**（对齐 `Object.is`，无 epsilon、`NaN != 一切`）。
- 时间窗：任一存在时 `fj.timestampMs` 缺 → 不匹配；否则 `from ≤ timestampMs ≤ to`。
缺省轴不应用；各轴 AND。**用共享测试向量**（把 `canfilter.test.ts` 7 例逐字搬为 Rust）钉住一致（Review Focus 1）。

## 5. 后端接口（`lumelog-app`）
- 抽 `fn build_frame_source(kind:&str, regex:&Option<String>, lines:DocLines) -> Result<Box<dyn FrameSource>, IpcError>`（candump/asc/regex；regex 编译失败 `bad_regex`/`regex_compile`，未知 kind `source_format`）；`can_parse` 改调用它（行为不变、去重）。prologue 仍用一次性 `RegexSource::new(empty, p)` 预编译校验（同 `can_parse`），`build_frame_source` 在 worker 内使用。
- DTO（camelCase）：`CanFilterJson`（见 §4）、`CanExportStats { written:u64, scanned:u64, cancelled:bool, errored:bool }`、`CanExportProgress { written:u64 }`。
- `#[tauri::command] export_csv(state, app:AppHandle, doc_id, dbc_id:Option<u64>, source_kind:String, regex:Option<String>, filter:CanFilterJson, target_path:String, on_progress:Channel<CanExportProgress>, on_done:Channel<CanExportStats>) -> Result<u64,IpcError>`
- `#[tauri::command] export_jsonl(同上参数) -> Result<u64,IpcError>`
- `#[tauri::command] export_can_cancel(state, request_id:u64) -> Result<(),IpcError>`（复用 `begin_search/end_search/cancel` 注册表）。
- 共用扫描核 `fn export_can<W:Write>(doc:&mut DocFile, dbc:Option<&Database>, kind:&str, regex:&Option<String>, filter:&CanFilterJson, writer:&mut W, on_progress:&mut dyn FnMut(u64), cancel:&dyn Fn()->bool, wide:bool) -> CanExportStats`：`build_frame_source` 建源→逐 `next_frame`：`frame_json(&f, dbc)` 得一个 `FrameJson`（**解码一次**）→ `frame_matches` 命中则写行（CSV：meta + 按 `消息名.信号名` 从 `fj.signals[].physical` 填列；JSONL：`serde_json::to_string(&fj)`+`\n`）；每帧判 `cancel`；每满一批 `on_progress(written)`；`scanned` 计解码帧总数。读/写错吸收为 `errored` 停止（不返 Result，同 `export::run`）。
- 命令体：`begin_search`；prologue `state.get(doc_id)`→`unknown_doc`、regex 一次性预编译校验、`dbc_id.and_then(get_dbc)`、`File::create(target_path)` 失败即 `fail(io)`（**不建 worker、不空写**）；`spawn_blocking` 持 `doc_arc.lock()` 跑 `export_can`（`BufWriter`）→ `flush` → `on_done` 必发 → `end_search`；早返 `req_id`。`lib.rs` 注册三命令。

## 6. 前端
- `ipc.ts`：`exportCsv`/`exportJsonl`（args `{docId,dbcId?,sourceKind,regex?,filter:CanFilter,targetPath,onProgress,onDone}: Promise<number>`，多 Channel）+ `exportCanCancel(requestId)`；`CanExportStats` 类型。`CanFilter` 直接复用 canfilter.ts 的接口（字段与 `CanFilterJson` camelCase 对齐）。
- `canpanel.ts` 工具条加两按钮"导出 CSV…""导出 JSONL…"→回调 `opts.onExport(kind)`；`main.ts` 处理：`save` 对话框（默认 `<docname>.csv`/`.jsonl`）→ 取 dock 当前 `sourceKind/canRegexInput.value/lastDbcId/该 doc 的 CanFilter` → `exportCsv/exportJsonl`，`canExportBusy`+`canExportReqId` 复用与解析互斥的态（进行中→"取消"调 `exportCanCancel`），进度/完成 toast。**导出用的 `filter` 即面板当前 `canF` 组装的 `CanFilter`**（与显示一致）。

## 7. 错误 / 边界
- 非法正则/未知源 → 同步 `bad_regex`/`source_format`（不空写、不建文件）。未知 doc → `unknown_doc`。目标路径不可写 → `io`（prologue）。
- 取消：保留已写前缀，`on_done{cancelled:true}`；读写错 `errored`；`end_search` 每路径恰一次。
- 无 DBC：CSV 仅 meta；信号条件过滤因无解码 → 不匹配（结果为空，与前端 signals 空一致）。
- 远程帧/错误帧：`frame_json` 已处理（无数据/空信号），CSV 信号列空、JSONL 原样。
- 扩展帧 id：沿用 `frame_json`/`can_parse` 现有 id 语义（增量2 park 项不改）。

## 8. 全局约束（沿用）
简体中文 UI；DTO camelCase + Option `skip_serializing_if`；命令 camel↔snake 自动；`char_map`/全文不驻留（逐帧流式写）；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；无空格 commit 消息；commit 后 push；分支 `--no-ff` 合 main。

## 9. 验收
**可测：**
- `frame_matches`：`canfilter.test.ts` 7 例语义在 Rust 全复现（ID 闭/反、msgName、信号五运算含 `Eq` 精确、缺时间戳、缺信号不匹配、缺省轴不应用）。
- `export_can`（tempfile 目标）：夹具 `candump_basic.log`+`sample.dbc` → CSV 表头=meta+DBC 信号列、行物理值正确、过滤生效；JSONL 每行合法且字段同 `frame_json`；取消早停 + `errored` 路径。
- `build_frame_source` 抽取后 `can_parse` 行为不变（既有 CAN 测不回归）。
- `ipc.ts`：`exportCsv/exportJsonl/exportCanCancel` typeof + 透传 camelCase 参数。
**GUI 手工（`tauri dev`）：**CanPanel 解析→设 ID/信号/时间过滤→"导出 CSV…"→Excel 打开宽表列/值正确；JSONL 行数=可见帧；导出中取消→文件前缀。

## 10. 分期（计划切分预告）
① `frame_matches`+`CanExportFilter`（Rust 端口）+ 共享测试向量 → ② 抽 `build_frame_source`（`can_parse` 改用）→ ③ `export_can` 扫描核 + CSV/JSONL 行构造 + 单测 → ④ `export_csv`/`export_jsonl`/`export_can_cancel` 命令 + DTO + 注册 + 集成测 → ⑤ `ipc.ts` 桥 → ⑥ CanPanel 导出按钮 + `main.ts` 流程（复用 CanFilter/互斥态）→ ⑦ 收尾 + 全量四门 + 终审 + 合 main。

## 11. 关键取舍（已定案）
- 后端重解码（你选）+ 过滤在 Rust 复现（端口），以共享测试向量钉与 `canfilter.ts` 一致；代价：改 canfilter.ts 需同步。
- CSV 列集取自 DBC 故可流式、不缓冲全帧；`消息名.信号名` 限定去重；物理值 only（raw/枚举走 JSONL）。
- JSONL 复用 `frame_json` 零新逻辑；`export_can` 与 `export_lines`/`can_parse` 同壳（prologue+`spawn_blocking`+持 doc 锁整轮+必发 on_done/`end_search`）。
