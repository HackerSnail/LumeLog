# Roadmap-2：BLF（Vector Binary Logging Format）报文源设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` Roadmap；承接：`lumelog-can` 的 `FrameSource`/`Frame`/`SourceStats`、CanPanel（`FrameStore`/`CanFilter`/DBC 解码）、`export_can`、`parse_frames`、`can_cancel`。格式以 python-can `blf.py` 与 `blf_asc 0.2.0` 实现为参照（BLF 官方文档未公开）。

**Goal:** 直接打开 `.blf`（文件对话框），流式解析 CAN 与 CAN-FD 帧进现有 CanPanel（过滤/信号解码/停止复用），并支持把当前视图过滤后的帧导出为 CSV 宽表 / JSONL。无源文本（行号=帧序号，跳编辑器禁用）。

**Architecture:** `lumelog_can::blf::BlfSource` 实现 `FrameSource`：解析 `LOGG` 文件头、顶层 `LOG_CONTAINER`（无压缩或 zlib）、容器内 `LOBJ` 对象（v1/v2 头），产出 `Frame`；容器边界可切开对象，跨容器用 tail 缓冲续接。app 新增 `can_parse_blf`（复用 `IpcError::from_can`/`parse_frames`/`can_cancel`）与 `can_export_blf_csv/jsonl`（先把 `export_can` 的帧→行逻辑抽成 `export_frames`，BLF 与文本源共用）。

**Tech Stack:** Rust：新增 `flate2 = "1"`（zlib，MIT/Apache）仅用于容器解压；TypeScript 复用现有 CanPanel/FrameStore。门：四门不变。

## 1. 范围与非目标
- 做：BLF 文件头 + `LOG_CONTAINER`（method 0 无压缩 / 2 zlib）；对象类型 **1 CAN_MESSAGE、86 CAN_MESSAGE2、100 CAN_FD_MESSAGE、101 CAN_FD_MESSAGE_64**；对象头 v1(16B 扩展) 与 v2(24B 扩展)；时间基准 = 文件头 `SYSTEMTIME` + 相对时间戳（1ns 或 10µs flag）；zlib 容器；跨容器分割对象；DBC 信号解码；视图过滤；停止解析；BLF 导出 CSV/JSONL。
- 不做：LIN/FlexRay/MOST/以太网/错误帧对象（计 skipped）；非小端；无压缩以外的压缩算法；BLF 写入；CLI 子命令；把 BLF 当作文本编辑器文档。

## 2. core `lumelog-can/src/blf.rs`
- 常量：`OBJ_HEADER_BASE=16`（`LOBJ`, u16 header_size@4, u16 header_version@6, u32 object_size@8, u32 object_type@12）；容器数据头 16B（`compression_method u16@0`，`uncompressed_size u32@8`）；对象类型 1/86/100/101；容器 10；`TIME_TEN_MICS=0x1`、`TIME_ONE_NANS=0x2`；`CAN_MSG_EXT=0x8000_0000`；`REMOTE=0x80`、`DIR=0x01`（classic flags）；FD flags：EDL/BRS/ESI；FD64 fd_flags：RTR=0x10、EDL=0x1000、BRS=0x2000、ESI=0x4000。
- 上限：`MAX_OBJECT_SIZE = 64 MiB`、`MAX_CONTAINER_DECOMPRESSED = 256 MiB`（防损坏 size 造成 OOM；超限→SourceFormat）。
- 打开：读 72B 文件头结构；magic 必须 `LOGG`；`header_size` ∈ [72, 1 MiB] 且跳到该偏移；`SYSTEMTIME` 位于 40..56（year,month,weekday,day,hour,min,sec,ms，各 u16）→ epoch 秒（Hinnant civil-days 算法；全 0/月日非法 → 无基准）。
- `next_frame`：pop pending；否则循环读顶层 16B 对象头（EOF→None）：非 `LOBJ`/size<16/超限→`SourceFormat`；读余下 `object_size-16` 字节并跳过 `object_size%4` 填充；仅 `LOG_CONTAINER` 进入解压（method 0/2；zlib 用 `ZlibDecoder` + `take(MAX_CONTAINER_DECOMPRESSED)`，读到上限未见 EOF→错误）；否则 `skipped+=1`。容器 payload 追加到 `tail`，按“前 8 字节内找 `LOBJ`”同步、解析完整对象，剩余不完整对象留在 `tail` 等下一容器。
- 内层对象：读 base + `header_version` 1(16B)/2(24B) 扩展（flags u32@+0、timestamp u64@+8；v2 其余忽略）；未知版本→跳过（skipped）。时间：`factor = if flags&TIME_TEN_MICS!=0 && flags&TIME_ONE_NANS==0 {1e-5} else {1e-9}`；`timestamp = start_epoch.unwrap_or(0.0) + rel*factor`（`Some`；无基准时即相对秒）。
- 帧映射：
  - **1/86（classic）**：payload≥16：channel u16@0（BLF 从 1 计，`Frame.channel = saturating_sub(1)` 为 u8）、flags u8@2、dlc u8@3、id u32@4（`&0x1FFF_FFFF`，ext=bit31）、data 8B@8（取 `min(dlc,8)`，Frame.data 其余补 0）；RTR=flags&0x80、DIR→is_rx=false；is_fd=false。
  - **100（CAN FD）**：payload≥84：channel u16@0、flags u8@2、dlc_code u8@3、id u32@4、fd_flags u8@13、valid_bytes u8@14、data 64B@20；`Frame.dlc = dlc2len(dlc_code)`、data 取 `min(valid,64)`；is_fd=fd_flags&0x1、brs&0x2、esi&0x4；RTR 同 classic。
  - **101（CAN FD 64）**：payload≥40：channel u8@0、dlc_code u8@1、valid u8@2、id u32@4、fd_flags u32@12、direction u8@34（0=Rx）、ext_data_offset u8@35、data@40（40B 结构后紧跟数据）；可用字节 = `ext_data_offset!=0 ? ext_data_offset.saturating_sub(56) : payload.len()-40`（payload 起点=对象偏移 16），长度 `min(valid, 可用, 64)`；is_fd/brs/esi 用 fd_flags 位；RTR=fd_flags&0x10。
  - 其它类型：`skipped += 1`；损坏对象→`SourceFormat` 并停止（记录 error）。
- `Frame.line_number = 帧序号（1 起）`；`stats.decoded/skipped`；`pub fn take_error(&mut self)->Option<CanError>`；实现 `FrameSource`。
- 测试夹具（自写 writer，仅供单测）：构造文件头（已知 SYSTEMTIME）、v1 对象、无压缩/zlib 容器、跨容器切分、未知对象；覆盖：classic 字段/时间（1.5s）、ext/RTR/channel、FD100（dlc 9→12、edl/brs/esi）、FD64（64B、direction/RTR）、压缩等价、跨容器只出一帧、10µs flag、未知对象计 skipped、坏 magic/截断/超限报错、帧序号递增。

## 3. app（`app/src-tauri`）
- `can_export.rs`：抽出 `pub fn export_frames<W: Write>(src:&mut dyn FrameSource, dbc, filter, writer, on_progress, cancel, wide) -> CanExportStats`（现 `export_can` 的循环/表头/统计原样迁移）；`export_can` 变为“建 `DocLines`+`build_frame_source` → 调 `export_frames`”，行为与测试不变。
- DTO：`BlfStatsJson { decoded: u64, skipped: u64, error: Option<String> }`（camelCase）。
- 命令：
  - `can_parse_blf(app, path, dbc_id, on_batch: Channel<Vec<FrameJson>>, on_done: Channel<BlfStatsJson>) -> Result<u64, IpcError>`：`begin_search` → `BlfSource::open(path).map_err(|e| fail(IpcError::from_can(e)))` 同步失败（坏文件直接 reject）→ `spawn_blocking`：`parse_frames(&mut src, db, emit, cancel)`；完成后 `src.take_error()` 写入 `error`，发 `on_done`，`end_search`。取消复用 `can_cancel`。
  - `can_export_blf_csv/jsonl(state, app, path, dbc_id, filter, target_path, on_progress, on_done, wide)`：同 `run_can_export` 骨架（`begin_search`、`open_export_target`、`BlfSource::open` 前置校验），worker 用 `export_frames`；若 `take_error()` 有错则 `errored=true`。
- 注册三命令。

## 4. 前端
- `ipc.ts`：`BlfStats{decoded,skipped,error:string|null}`、`canParseBlf(args)`、`exportBlfCsv/exportBlfJsonl(args)`（`docId` 换成 `path`）。
- `main.ts`：BLF 模式（无文档）：
  - 状态：`const BLF_KEY = -2;` `let blfView: { path: string; store: FrameStore } | null = null;` 过滤器复用 `canFilters`（键 `BLF_KEY`）。
  - 工具栏新增“打开 BLF…”（`.blf` 对话框）→ `startBlfParse(path)`：确保 dock 打开、`blfView = {path, store}`、`store.clear()`、attach、`setFilter(canFilters.get(BLF_KEY))`、`restoreFilterInputs`、`setCanParsing(true)`，调用 `canParseBlf`（`onBatch` push+render；`onDone` toast `BLF 解码 N 帧，跳过 M 个对象`，有 `error` 再 toast 一次，`setCanParsing(false)`）。
  - `switchCanView`/`toggleCanDock`：若 `blfView` 存在 → attach BLF store + BLF_KEY 过滤；否则原文档视图。
  - `onCanRunClick`：BLF 模式下“重新解析 BLF”/“停止”（复用 `canReqId`+`can_cancel`）。
  - `applyFilterFromInputs` 用当前视图键（BLF 模式 → `BLF_KEY`）。
  - `revealSourceLine`：BLF 模式直接 toast“BLF 无源文本，无法跳转到行”。
  - `tabs.onActivate(id>=0)`：`blfView = null;`（切回文档即退出 BLF 视图）并在 dock 可见时 `switchCanView()`。
  - 导出：BLF 模式下 `onCanExport` 改用 `exportBlf*`（`path=blfView.path`），默认文件名取 BLF 基名；进度/取消/统计文案沿用现有。
  - `setCanParsing` 文案按模式：“解析中…”/“解析报文” ↔ BLF：“解析中…”/“重新解析 BLF”。
- 不改 CanPanel 本身（`onRowActivate` 回调由 main 决定 BLF 下 no-op）。

## 5. 错误 / 边界
- 坏 magic/截断/对象超限/解压超限/内层对象损坏 → `source_format`（解析中出错走 `BlfStats.error` + toast，表格保留已解析帧）。
- 停止：协作取消（`can_cancel`），已收帧保留，`onDone` 仍发。
- 内存：顶层对象 ≤64 MiB、解压 ≤256 MiB、容器逐块解压；不整文件载入。
- 时间：文件头无有效时间 → 相对时间（从 0 起）；文档注明。
- 无 DBC → 仅报文表；选中 DBC 后重新解析即有信号列。
- 导出与解析共享 busy 单飞；BLF 与文档视图互斥，切标签退出 BLF 视图；再次打开 BLF 重新解析（不复用旧表）。

## 6. 验收
- **core 单测**：见 §2（夹具生成，覆盖压缩/跨容器/FD/FD64/时间/错误）。
- **app**：`export_can` 既有测试保持通过（重构不回归）；`can_export_blf` 的 helper 测试（可选：临时 BLF 导出行数/取消）。
- **前端**：`ipc` 新包装 typeof；`npm test` 不回归。
- **GUI 手工**：打开压缩/未压缩 BLF → 表格出现、帧数/跳过统计正确；DBC 导入后信号可见；ID/消息/时间过滤生效；停止后保留部分；切标签回文档视图；BLF 导出 CSV/JSONL 与表格过滤一致；坏文件报错不崩。

## 7. 全局约束
简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；push；`--no-ff` 合 main。分支基 `main`（当前 `b908840`）→ `feat/roadmap-blf`。

## 8. 分期（计划预告）
① core `blf.rs`（读取器+夹具测试，加 flate2）→ ② `export_frames` 重构（既有测试不回归）→ ③ app 解析/导出命令 + DTO + 注册 → ④ 前端 ipc → ⑤ `main.ts` BLF 视图/解析/过滤/导出 → ⑥ 四门+终审+合 main。

## 9. 关键取舍（已定案）
- **自带解析器**而非依赖第三方 crate（`blf_asc` 等仅作格式参照）：与项目既有 candump/ASC 解析一致、可测、可控；唯一新依赖 `flate2`（zlib）。
- **对象范围** CAN/CAN-FD + 容器 zlib（你选）；其它对象跳过计数。
- **直接进 CanPanel**（你选）：无源文本，行号=帧序号，跳转禁用，DBC/过滤/停止全复用。
- **本轮做导出**（你选）：`export_frames` 泛化，BLF 与文本源共用行构造/过滤。
- 时间基准用文件头 SYSTEMTIME + 相对时长（1ns/10µs），无基准则相对秒。
