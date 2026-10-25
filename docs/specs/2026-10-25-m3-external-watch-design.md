# M3 子项目 B2：外部修改持续监听 设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` §4.4（持续外部修改监听：`notify` debounce 500ms；干净文档自动重载保滚动；脏文档标题黄点 + 非阻断提示，保存仍走三选；监听异常降级为仅保存前检查并状态栏标注）、§15 A10、§4.1（`Document` 持 `size+mtime` 作为外部修改检测基线）。承接 M1–M3（`DocFile::disk_meta()`、`save.rs::disk_meta(path)`/冲突门、`doc_open`/`doc_close`、`tabs` 脏点、`ListView` 虚拟列表、恢复横幅交互样式）。

**Goal:** 已打开文档（编辑/只读）被外部修改时：干净文档自动重载（保留光标/滚动），脏文档黄点 + 非阻断提示并可"重新加载/保留"；保存冲突弹三选；文件被删/改名与监听降级都有明确降级路径。

**Architecture:** 后端新 `app/src-tauri/src/watch.rs`：`notify::RecommendedWatcher` **监听文档父目录**（抗 rename 替换），路径→文档注册表 + 500ms 去抖线程；事件按 `disk_meta` 与文档快照比对**抑制自我写入**，经单个 `Channel<ExternalEvent>` 推前端。`doc_open` 成功即 `watch_path`、`doc_close` 前 `unwatch`。新命令 `doc_reload(doc_id)`：按 `path` 重开 `DocFile` 并**原地替换同一 `Arc<Mutex<DocFile>>` 内的实例**（doc_id/标签不变、索引重建），返回新 `DocInfo`。前端启动订阅事件；干净文档自动重载；脏文档置黄点 + 提示条；保存三选；监听降级写状态栏。

**Tech Stack:** Rust（新增 `notify` 6.x `RecommendedWatcher`；去抖线程用 `Condvar::wait_timeout`）+ TypeScript 原生 DOM。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

## 1. 范围与非目标
- 做：双模式（编辑+只读）监听；父目录监听 + 500ms 合并；自我写入抑制；干净自动重载（编辑保光标/滚动，只读保滚动+清查询）；脏文档黄点 + 提示条（重新加载/保留）；保存冲突三选（重新加载/强制覆盖/取消）；删除/改名事件；监听降级（状态栏标注）。
- 不做：tail 跟随/自动滚动到底（Roadmap）、未打开文件/目录树监听、多窗口、跨设备。

## 2. 后端设计（`watch.rs`）
- 依赖：`notify = "6"`（`RecommendedWatcher` + `RecursiveMode::NonRecursive`）。
- **纯逻辑（可测）**：
  - `pub enum ExternalKind { Changed, Removed }`；`pub fn classify(disk: Option<(u64, SystemTime)>, snapshot: (u64, SystemTime)) -> Option<ExternalKind>`：`disk==snapshot`→`None`（自我写入/无变化）；`None`→`Some(Removed)`；否则 `Changed`。
  - `pub struct WatchRegistry { dirs: HashMap<PathBuf,u32>, paths: HashSet<PathBuf> }`：`watch(path)->Option<PathBuf /*新目录*/>`、`unwatch(path)->Option<PathBuf /*可停止监听的目录*/>`（父目录 refcount 0 时返回）；`tracked(&self, path)->bool`。
- **运行态** `AppWatch`（`app.manage`）：
  - `Mutex<Inner>`：`watcher: Option<RecommendedWatcher>`、`registry: WatchRegistry`、`channel: Option<Channel<ExternalEvent>>`、`pending: HashSet<PathBuf>`、`degraded: Option<String>`、`stop: Arc<(Mutex<bool>, Condvar)>`。
  - 去抖线程（`watch_subscribe` 首次启动）：`Condvar::wait_timeout(500ms)` 静默后 drain `pending`；对每个 path：`fs::metadata` → `classify(disk_meta, doc_snapshot)`（从 `AppDocs` 找到该 path 的 `DocFile` 快照；多 doc 同 path 逐个比对）；非 `None` → `channel.send(ExternalEvent{path, kind})`。
  - watcher 回调：`pending.insert(event.path)` + `Condvar.notify_one()`。
- **命令**：
  - `watch_subscribe(on_event: Channel<ExternalEvent>) -> Result<WatchStatus{degraded: Option<String>}, IpcError>`：初始化 watcher（失败→`degraded=Some(原因)`，不返错，前端标注）；绑定通道。若注册表里已有（subscribe 前被 `watch_path` 记下的）目录，创建时一并加入监听。
  - `watch_status() -> Result<WatchStatus, IpcError>`：读当前降级状态（打开新目录可能才失败，前端在打开后刷新）。
  - `doc_open` 成功后：`AppWatch::watch_path(path)`（失败仅记 degraded，不影响打开）。
  - `doc_close(doc_id)`：先取 path 再 `unwatch_path`（`doc_close` 现有签名只吃 doc_id，需先 `state.get` 取 `doc.path()`，再 `state.remove`）。
  - `#[tauri::command] doc_reload(state, doc_id, on_progress: Channel<IndexProgress>) -> Result<DocInfo, IpcError>`：`state.get`→取 path→`validate_open_path(path, SIZE_LIMIT)`（超限→`too_large`）→`DocFile::open`→`AppDocs::replace(doc_id, fresh)`（**锁同一 Arc 原地赋值**，令既有 `DocLines`/导出持有的 Arc 看到新文件）→ 返回新 `DocInfo`（同 doc_id，`mode` 可能变）+ 单次完成 tick。`AppDocs::replace` 新增：`get(id)` 后 `*arc.lock() = doc; true`。
- **自我写入抑制**：`doc_save` 的 `reload_after_save` 已刷新 `(size,mtime)`；`classify` 与文档快照相等 → 忽略。外部"同 size 同 mtime"改写接受为理论遗漏。

## 3. 前端
- `ipc.ts`：`ExternalEvent{path,kind:"changed"|"removed"}`、`WatchStatus{degraded:string|null}`；`watchSubscribe(onEvent):Promise<WatchStatus>`；`docReload(docId,onProgress):Promise<DocInfo>`。
- `tabs.ts`：`TabEntry` 增 `ext` span（`.ext-dot`，class `warn`）；`setExternalFlag(docId, on)`（文字 `●` + class 切换，独立于 `.dirty-dot`）。
- `listview.ts`：新增 `scrollTop(): number` 与 `setScrollTop(n: number)`（转发内部滚动容器）。
- `statusbar.ts`：新增 `setWatch(state:"ok"|"degraded", reason?:string)`（一段小文本/图标，degraded 时带 tooltip）。
- `main.ts`：
  - 启动 `watchSubscribe` → 保存 `watchState`；降级→`status.setWatch("degraded", reason)`；`path→docId` 由 `docStates` 反查（可能多 doc）。
  - 事件处理：
    - `removed`：置该 doc 黄点（脏/干净都算"外部已删"），toast"文件已被外部删除：…"；保存自然走 `io` 错误。干净文档不自动重载。
    - `changed` + **干净**：`docReload(docId, onIndexProgress)` →
      - 编辑：记 `scrollTop = handle.view.scrollDOM.scrollTop`、`selection = view.state.selection.main`；`loadAllText(docId, totalLines)`→`handle.replaceText(text)`→`handle.markSaved()`→恢复 clamped selection + scroll；`ds.encoding/eol/invalid/totalLines` 取 `DocInfo`；`tabs.refreshDirty`；toast"文件已变化，已重新加载"。
      - 只读：记 `scroll = list.scrollTop()`；`list.invalidate()`+`list.setView(null,false,false)`；清 `queryHits/Cursor`；`ds` 更新；`list.refresh()`；`list.setScrollTop(min(scroll, maxScroll))`；toast"文件已变化，已重新加载（查询需重新应用）"。
      - `DocInfo.mode` 与当前 `ds.mode` 不同 → 关闭该标签（无脏询问，因干净）并 `openPath(path)` 重开（toast 说明）。
      - 重载期间 `reloading` 标志（按 docId）防重入；完成后若 `pendingAgain` 则再重载一次（最多 1 次追赶）。
    - `changed` + **脏**：`tabs.setExternalFlag(docId,true)` + 显示外部修改提示条（`.ext-bar`，含"重新加载（丢弃我的修改）"/"保留我的修改"）；保留→仅关提示条，黄点留到保存/关闭。
  - **重新加载（脏文档，用户确认丢弃修改）**：与干净编辑重载同一路径——`docReload` → `loadAllText` → `replaceText` → **`markSaved()` 回到干净态**（`replaceText` 会置脏，随后清掉，因为内容就是磁盘内容）→ 清黄点/提示条。
  - **保存冲突三选**：仅 `IpcError.code === "conflict"` 时改调自绘 `confirmExternalConflict(): Promise<"reload"|"overwrite"|"cancel">`（DOM 模态，三键，Esc=取消）。`reload`→执行上述重载（丢弃内存编辑）；`overwrite`→现有 `force:true` 保存；`cancel`→复位 busy。`unmappable` 仍用现有两键 `ask`。保存成功→`tabs.setExternalFlag(docId,false)` + 隐藏提示条。
  - **Save-As**：`doc_save` 成功后**后端**重绑 `DocFile` 到新路径（path/index/cache/disk 快照随之更新，冲突基线不再锚在旧文件）并 `retarget(old,new)`（在释放 doc 锁后调用，避免与 watcher 回调抢锁）；前端无需额外调用。
  - 关闭文档/编码切换（丢弃脏改动）后 → 清黄点/提示条。
- `styles.css`：`.ext-dot.warn`（黄点）、`.ext-bar`（底部/顶部非阻断条，复用 banner 风格）、`.modal-3way` + 按钮。

## 4. 错误 / 边界
- 自我写入：`(size,mtime)` 比对抑制（快照与当前磁盘在同一 doc 锁内成对读取，避免保存交错误报；路径按规范化比较兼容 Windows 大小写/分隔符）。
- 文件删除：`Removed` 事件；保存 → `io` 错误（既有）。
- watcher 创建失败/网络盘：`degraded` → 状态栏标注；保存前冲突检查仍在。
- 只读大文件重载：同步重建索引（`doc_open` 同为同步，~0.6s/GB）；期间单飞。
- `doc_reload` 遇 `too_large`：返回 `too_large`，前端提示并保留旧文档。
- 模式跨越 200MB 阈值：前端关闭+重开（罕见）。

## 5. 验收
**可测（Rust）：**
- `classify`：相等→None；size 或 mtime 变→Changed；None→Removed。
- `WatchRegistry`：两 path 同目录只 watch 一次；refcount 归零才返回可停止目录；`tracked`。
- `AppDocs::replace`：同 id 替换后 `get` 返回新实例；未知 id 返回 false。
- `doc_reload`（集成，tempfile）：改文件内容/行数后 reload → 返回新 `totalLines`，`doc_chunk` 读到新文本，`disk_meta` 已更新。
**前端：**
- `tabs.setExternalFlag`（伪 view）：黄点 class 切换、与 `refreshDirty` 独立。
- `ipc` 包装 typeof（watchSubscribe/docReload/watchStatus）。
- 纯 helper（如需）：`clampScroll(prev, newMax)`。
**GUI 手工：**只读日志 `echo >>` 追加→自动重载且滚动保持；编辑干净→外部覆盖→文本更新、光标近似保持；编辑脏→外部覆盖→黄点+提示条→保存弹三选（三种分支）；删除文件→提示；降级状态栏。

## 6. 全局约束（沿用）
简体中文 UI；DTO camelCase；命令驼峰↔下划线自动；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；无空格 commit 消息；commit 后 push；分支 `--no-ff` 合 main。

## 7. 分期（计划切分预告）
① `watch.rs` 纯逻辑（`classify`/`WatchRegistry` + 测）→ ② `AppWatch` 运行态 + watcher/去抖线程 + `watch_subscribe` + doc_open/doc_close 挂钩 + `watch_retarget` → ③ `AppDocs::replace` + `doc_reload` 命令 + 集成测 → ④ `ipc.ts` 三包装 + `tabs.setExternalFlag` + `listview` 滚动存取 + `statusbar.setWatch` → ⑤ `main.ts` 事件分发（干净自动重载/脏黄点+提示条/删除/降级）→ ⑥ 保存三选弹窗 + Save-As retarget + 样式 → ⑦ 收尾四门 + 终审 + 合 main。

## 8. 关键取舍（已定案）
- **父目录监听 + 路径过滤**（非文件监听）：抗 rename 替换。
- **自我写入用 `(size,mtime)` 快照比对**（非时间窗抑制）：确定、无盲区。
- **`doc_reload` 原地替换同一 `Arc` 内 DocFile**（非换 Arc/换 doc_id）：既有 `DocLines`/导出持有的 Arc 立即看到新文件，标签/文档 id 稳定。
- **订阅用单个 `Channel`**（单窗口），事件按 path 由前端反查 docId。
- **脏文档保存三选自绘模态**（`ask` 只有两键），符合 spec。
- **降级=状态栏标注 + 保存前检查兜底**。
- 只读重载同步重建索引（与 `doc_open` 一致），单飞防重入。
