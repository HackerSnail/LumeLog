# Roadmap-1：tail 跟随 + 滚动截断（只读）设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` Roadmap（“监控 tail 与滚动截断”）、§4.1/§4.3（行索引/只读虚拟列表）。承接已交付：`LineIndex{offsets,file_size}`、`DocFile::{disk_meta,reindex}`、B2 外部监听（`notify` 500ms 去抖 + 只读干净自动重载）、`doc_reload`、`ListView` 虚拟列表（`scrollTop/setScrollTop`）、`Statusbar`。

**Goal:** 只读打开的大日志在外部**追加**时增量扩展行索引并自动滚到底部（可手动开关）；用户上滚即暂停并提示“回到底部”；**截断/轮转**时回退全量重载并保持跟随；编辑模式不跟随。

**Architecture:** core 新增 `LineIndex::extend`（只扫追加字节的 `0x0A`）+ `DocFile::try_append`（stat → 头签名/size 判轮转 → 增量 extend → **刷新 `(size,mtime)` 快照**，避免 B2 watcher 反复报同一增长）。app 新增 `doc_tail_append` 命令返回 `{rotated|linesAdded,totalLines,size}`。前端只读视图加“跟随尾部”开关 + `isNearBottom` 暂停/恢复；外部 `changed` 在只读时优先走增量，`rotated` 再走 `doc_reload`。

**Tech Stack:** Rust（无新依赖；FNV-1a 头签名手写）+ TypeScript。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

## 1. 范围与非目标
- 做：只读跟随开关（状态栏，按 docId 记忆）；增量追加（索引扩展 + 行数更新 + 可选滚底）；滚离暂停 + “回到底部”；截断/轮转全量重载；头签名轮转检测；快照即时刷新。
- 不做：编辑模式跟随；平滑滚动动画；非 notify 文件系统的轮询兜底（沿用 B2 降级）；tail 的“只显示末尾 N 行”截断窗口（本项“滚动截断”指文件被截断/轮转，不新增视图窗口）。

## 2. core（`crates/lumelog-core`）
- `LineIndex`：
  - `pub fn extend(&mut self, appended: &[u8], base_offset: u64) -> Result<(), CoreError>`：扫描 `appended` 每个 `0x0A`，偏移 `base_offset + i + 1` 推入；> `u32::MAX` → `CoreError::TooLarge`；`file_size = base_offset + appended.len()`。
  - 边界已核对：旧文件“尾部有 LF”（末尾空行偏移已存在）与“尾部无 LF”（续写同一行）两种情况都只需按上述规则推入，不产生重复起点。
- `DocFile`：
  - 新增私有 `head_sig: u64`：打开与 `reload_after_save` 时对前 `min(512, size)` 字节做 FNV-1a64；`set_encoding` 不动（字节未变）。
  - `pub enum AppendOutcome { NoChange, Appended { lines_added: u64, total_lines: u64 }, Rotated }`
  - `pub fn try_append(&mut self) -> Result<AppendOutcome, CoreError>`：
    1. **从路径新开探测句柄**（`File::open(path)`）并 `stat`：旧句柄在 `mv` 轮转后仍指向旧 inode，不能用于判定；`new_size < self.size` → `Rotated`；`new_size > u32::MAX` → `TooLarge`（与 open 对齐）；
    2. 用探测句柄读新头的前 `head_len` 字节（**打开时冻结的前缀长度**，±增长不改变哈希窗口），签名 != `head_sig` → `Rotated`（覆盖等大/更大的 rename 替换）；
    3. `new_size == self.size` → `NoChange`；
    4. 按 `READ_CHUNK`(8MB) 分块读 `[old_size,new_size)` 并逐块 `index.extend`；成功后将 `self.file` **重绑为探测句柄**（索引与新 inode 一致），更新 `size/mtime` 并重算头签名 → `Appended{lines_added,total}`。
  - 缓存不清（旧行偏移不变）；新行由 `chunk_at` 按需读。
- 测试：`extend` 尾有/无 LF、空追加、多 LF、4GB 越界拒（**整块失败不留下部分偏移**）；`try_append` 追加后 `line_count`/`chunk_at` 新行、`NoChange`、截断→`Rotated`、同大小改写头部→`Rotated`、**rename 成更大新文件→`Rotated`**、追加后 `disk_meta` 刷新。

## 3. app（`app/src-tauri`）
- DTO：`TailAppendJson { rotated: bool, lines_added: u64, total_lines: u64, size: u64 }`（camelCase）。
- 命令 `doc_tail_append(doc_id) -> Result<TailAppendJson, IpcError>`：`state.get` → 持 doc 锁调 `try_append`；`Rotated` → `{rotated:true,..0}`（不改索引，由前端走 `doc_reload`）；`NoChange` → `{rotated:false,lines_added:0,total_lines:当前}`；`Appended` → 对应值。`CoreError::TooLarge` 由 `IpcError::from_core` 映射为 `too_large`。
- 可测 helper：`fn tail_append(state:&AppDocs, doc_id:u64) -> Result<TailAppendJson, IpcError>`（无 `State`）。
- `lib.rs` 注册。

## 4. 前端
- `ipc.ts`：`TailAppend` 类型 + `docTailAppend(docId):Promise<TailAppend>`。
- `listview.ts`：
  - `scrollToBottom(): void`（`root.scrollTop = maxScrollTop()`，触发自身 scroll/render）；
  - `maxScrollTop(): number`（`max(0, scrollHeight - clientHeight)`）；
  - `export function isNearBottom(scrollTop:number, maxScroll:number, threshold=48): boolean`（纯函数，可测）。
- `statusbar.ts`：
  - `setFollowAvailable(on:boolean)`：只读文档显示 `“跟随尾部”` 切换按钮（`.status-follow`），编辑文档隐藏；
  - `setFollow(on:boolean)`：按钮 `active` 态；
  - 回调 `onFollow?: (on:boolean)=>void`；点击切换。
- `main.ts`：
  - `const follow = new Map<number, { on:boolean; paused:boolean }>()`；`mountList` 时 `status.setFollowAvailable(true)` 并初始化 `{on:false,paused:false}`；`mountDoc`/切标签时 `setFollowAvailable(false)` 并恢复该 doc 的 follow 显示。
  - `onFollow(on)`：`follow.set(docId,{on,paused:false})`；`on` 时 `list.scrollToBottom()`；`status.setFollow(on)`。
  - ListView 订阅滚动（现有 `onScroll` 之外新增回调 `onScrollTop?(top)` 或 main 监听 `root` 的 scroll 事件）：只读且 `follow.on` 时，`!isNearBottom(top, maxScrollTop())` → `paused=true` + 显示 `.follow-resume`（“回到底部”，点击→滚底+`paused=false`）；回底→`paused=false` + 隐藏。**需要 `ListView` 暴露 `onScrollPosition` 回调（构造 opts 新增可选回调，滚动时带 `scrollTop/maxScroll`）**。
  - 外部 `changed` 且 `lists.has(docId)`：
    1. `const r = await docTailAppend(docId)`；
    2. `r.rotated` → 走现有 `reloadDocFromDisk(docId, path)`；完成后若 `follow.on` → `list.scrollToBottom()`；
    3. 否则（含 `NoChange`/`Appended`）→ 若 `size` 增长但 `linesAdded==0`（半行写入）→ `invalidate+refresh` 尾部；否则更新 `ds.totalLines`（`r.totalLines`）与 `indexed`、`list.refresh()`、活动文档同步 `status.setReadonly/updateInspector`；`follow.on&&!paused` → `scrollToBottom()`，否则保持 `scrollTop`；`r.linesAdded>0` 时按 **docId** 节流提示（≥1s，“已追加 N 行”）。
  - **编辑模式**仍走 B2 整档重载；只读一律先增量，follow 仅决定是否滚底。
- `styles.css`：`.status-follow`（active 高亮）、`.follow-resume`（复用 `.ext-bar` 风格，右下/底部居中）。

## 5. 错误 / 边界
- 追加过程中用户滚离：本次不滚底（`paused`），后续追加仍不滚底直至回底/点按钮。
- 截断/轮转：`rotated` → 全量 `doc_reload`（保留 follow：滚到新底部）。
- 文件删除：现有 `removed` 路径；`doc_tail_append` 的 `io` 错误 toast 一次并停跟随。
- 4GB：`extend` 越界 → `too_large`（同打开）。
- 快照：`try_append` 成功后刷新 `(size,mtime)`，B2 watcher 的 `classify` 立即视为无变化（不重复触发）。
- 隐藏标签：`scrollToBottom()` 在 `clientHeight==0` 时直接返回（否则会被夹到 0 丢失跟随位置）；切回时 `syncStatus` 若 `follow.on&&!paused` 再滚底。
- 跟随 chip 绑定 owner docId（切换/关闭只影响自己的 chip）；`Statusbar.clear()` 隐藏跟随按钮。
- 打开时为空（`head_len==0`）的文件：头指纹窗口为空，轮转退化为尺寸判断（缩小才识别）；首次增长按追加处理（tail 从空文件起可用）。
- 尾部半行写入（无 LF）：`extend` 不新增行；下次追加继续同一行，正确。
- 编辑模式：不显示开关、不跟随。

## 6. 验收
**core 单测：** 见 §2。
**app 单测：** `tail_append` helper（tempfile 追加→`lines_added/total` 正确、`NoChange`、截断→`rotated`、`disk_meta` 刷新）。
**前端：** `isNearBottom` 边界（0/max/阈值内/外）；`ipc` 包装 typeof；`statusbar.setFollowAvailable/SetFollow`（jsdom：只读显示、active 切换、点击回调）。
**GUI 手工：** 只读日志开跟随→外部 `echo` 追加（尾无/有换行两种）→自动滚底、行数递增、无整页闪烁；上滚→暂停+“回到底部”；回底恢复；`> file` 截断与 `mv` 轮转→重载并继续跟随；编辑文档无开关。

## 7. 全局约束（沿用）
简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；无空格 commit 消息；commit 后 push；分支 `--no-ff` 合 main。

## 8. 分期（计划切分预告）
① core `LineIndex::extend` + 测 → ② core `DocFile::try_append` + 头签名 + 测 → ③ `doc_tail_append` 命令 + DTO + helper 测 + 注册 → ④ 前端 `ipc.ts` + `listview` 滚动 API/`isNearBottom`/滚动回调 + 测 → ⑤ `statusbar` 跟随按钮 + 测 → ⑥ `main.ts` 跟随状态与 changed 分流 + 样式 → ⑦ 收尾四门 + 终审 + 合 main。

## 9. 关键取舍（已定案）
- **增量扩展而非全量重载**（你选）；成本与追加字节成正比。
- **头签名（FNV-1a64，`head_len` = 打开时的 `min(512,size)` 并冻结）检测轮转**：用路径新句柄读头，覆盖等大/更大的 rename 替换；打开时为空的文件窗口为空，轮转退化为尺寸判断。
- **探测句柄成功即重绑**：`try_append` 用 `File::open(path)` 探测并在成功后替换 `self.file`，保证索引与 inode 一致（旧句柄在 `mv` 后指向已删除 inode）。
- **快照即时刷新**：防止 B2 watcher 对同一次增长重复触发。
- **只读 + 手动开关、滚离暂停**（你选）；follow 状态按 docId 记忆。
- 跟随采用直接 `scrollTop`（虚拟列表逐帧渲染），不做平滑动画。
- 追加提示做 ≥1s 节流，避免高频 toast。
