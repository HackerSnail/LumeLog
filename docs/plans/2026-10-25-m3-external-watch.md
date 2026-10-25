# M3 子项目 B2：外部修改持续监听 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 已打开文档（编辑/只读）被外部修改时：干净自动重载（保光标/滚动），脏黄点 + 非阻断提示（重新加载/保留），保存冲突三选；删除/改名与监听降级有明确路径。

**Architecture:** 新 `app/src-tauri/src/watch.rs`：`notify::RecommendedWatcher` 监听父目录 + 500ms 去抖线程 + `(size,mtime)` 快照抑制自我写入，经单个 `Channel<ExternalEvent>` 推前端。`doc_open`/`doc_close` 自动 watch/unwatch；`watch_retarget` 处理 Save-As；`doc_reload` 原地替换同一 `Arc<Mutex<DocFile>>`。前端订阅事件：干净→重载（编辑 `loadAllText`+`replaceText`+`markSaved`；只读 `invalidate`+滚动存取）；脏→`tabs.setExternalFlag` 黄点 + `.ext-bar`；保存 `conflict` 走三选模态。

**Tech Stack:** Rust（`notify = "6"`，`Condvar` 去抖；`AppDocs::replace/snapshot_for_path`）+ TypeScript 原生 DOM。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm --prefix app/frontend test` / `run build`。

**Spec:** `docs/specs/2026-10-25-m3-external-watch-design.md`。承接：`DocFile::{path,disk_meta}`、`doc_open/doc_close`、`tabs`（脏点/`TabView`）、`ListView`（`root` 即滚动容器）、`Statusbar`、`loadAllText`。

## Global Constraints

- 监听单位=**父目录**（`RecursiveMode::NonRecursive`），按 registry 的 path 过滤；同目录多文档 refcount。
- 去抖 500ms（静默窗）；每批 drain 后逐 path `classify`：`disk==snapshot`→忽略（自我写入）；`None`→`Removed`；否则 `Changed`。
- `doc_reload` **原地替换同一 `Arc<Mutex<DocFile>>`**（`*arc.lock() = fresh`），doc_id/标签不变；超限→`too_large`。
- 前端：干净自动重载；脏黄点 + `.ext-bar`；保存仅 `conflict` 用三选（`unmappable` 保持两键 `ask`）；Save-As 后 `watch_retarget`；降级写状态栏。
- 简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；commit 消息无空格；commit 后 push；分支 `--no-ff` 合 `main`。
- 分支基：从 `main`（当前 `d96f98c`）新建 `feat/m3-external-watch`。

## Review Focus

1. **自我写入抑制**：`classify` 三分支；`doc_save` 后 `(size,mtime)` 已刷新 → 自家保存不触发事件；`Save-As` 后旧 path 不再监听、新 path 已监听。
2. **去抖与合并**：500ms 静默窗；批量 drain；watcher 错误不 panic（记 degraded）。
3. **原地重载**：`AppDocs::replace` 后既有 `Arc` 持有者（`DocLines`/导出）看到新文件；`doc_chunk` 返回新文本/新行数；`disk_meta` 更新（防重载后又被判"变化"）。
4. **滚动/光标保持**：编辑恢复 clamped 选区 + `scrollDOM.scrollTop`；只读 `scrollTop()`/`setScrollTop()` 恢复。
5. **三选语义**：`reload` 丢弃内存编辑回干净态；`overwrite` force 保存清黄点；`cancel` 复位 busy；仅 `conflict` 触发。
6. **删除/降级**：删除→黄点 + 提示，不自动重载；watcher 创建失败→`WatchStatus.degraded` + 状态栏，打开/保存仍可用。

---

## Task 1: `watch.rs` 纯逻辑（`classify` + `WatchRegistry`）

**Files:** Create `app/src-tauri/src/watch.rs`；Modify `app/src-tauri/src/lib.rs`（`mod watch;`）
**Interfaces:** `#[derive(Serialize, Deserialize)] #[serde(rename_all="lowercase")] pub enum ExternalKind { Changed, Removed }`；`pub fn classify(disk: Option<(u64,SystemTime)>, snapshot:(u64,SystemTime)) -> Option<ExternalKind>`；`pub struct WatchRegistry { dirs: HashMap<PathBuf,u32>, paths: HashSet<PathBuf> }`，`watch(&mut self, path:&Path) -> Option<PathBuf>`（父目录首次→Some）、`unwatch(&mut self, path:&Path) -> Option<PathBuf>`（refcount 归零→Some）、`tracked(&self, path:&Path)->bool`；`pub struct ExternalEvent { pub path: String, pub kind: ExternalKind }`（Serialize，字段 `path/kind`，kind 序列化为 `"changed"|"removed"`）。模块 `#![allow(dead_code)]`（Task2 接入后剥）。
- [ ] **Step 1：测试**：`classify_equal_ignores`；`classify_size_or_mtime_changed`；`classify_missing_removed`；`registry_same_dir_once`（两文件同 dir → 首次 Some、二次 None）；`registry_unwatch_refcount`（删一个不返回、删第二个返回 dir）；`tracked_true_false`。
- [ ] **Step 2：** `cargo test -p lumelog-app watch::` → FAIL。
- [ ] **Step 3：实现**（`SystemTime` 比较用 `==`/`!=`；registry 用 `path.parent()`，None 时退化为 path 自身）。
- [ ] **Step 4：** PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:watch-pure-logic` + push。

---

## Task 2: `AppWatch` 运行态 + 订阅 + doc_open/close/retarget 挂钩

**Files:** Modify `app/src-tauri/src/watch.rs`（`AppWatch`/`Inner`/去抖线程/`watch_subscribe` 内部 `start`）；Modify `app/src-tauri/src/commands.rs`（`watch_subscribe`/`watch_retarget` 命令；`doc_open` 成功后 watch、`doc_close` 前 unwatch）；Modify `app/src-tauri/src/state.rs`（`snapshot_for_path`）；Modify `lib.rs`（`app.manage(AppWatch::default())` + 注册 2 命令）；Modify `Cargo.toml`（`notify = "6"`，`Cargo.lock` 随提交）。
**Interfaces:**
- `AppDocs::snapshot_for_path(&self, path:&Path) -> Vec<(u64,(u64,SystemTime))>`（锁 docs 表→逐个 doc 锁读 `disk_meta`）。
- `AppWatch::default()`；`fn start(&self, app:&AppHandle, channel:Channel<ExternalEvent>) -> WatchStatus`；`fn watch_path(&self, app:&AppHandle, path:&Path)`；`fn unwatch_path(&self, app:&AppHandle, path:&Path)`；`fn retarget(&self, app, old:&Path, new:&Path)`。
- 命令 `watch_subscribe(app, on_event: Channel<ExternalEvent>) -> Result<WatchStatus, IpcError>`（`WatchStatus { degraded: Option<String> }`）；`watch_retarget(app, doc_id:u64, path:String) -> Result<(),IpcError>`（从 `state.get(doc_id)` 取旧 path）。
- `doc_open` 末尾 `app.state::<AppWatch>().watch_path(&app, Path::new(&path))`；`doc_close` 开头取 `doc.path().to_path_buf()` 后 `unwatch_path`。
- 去抖线程：`Condvar::wait` 等首个 pending → `wait_timeout(500ms)` 直到一次超时（静默）→ drain → `snapshot_for_path` 逐 doc `classify` → 任一非 None 则 `channel.send(ExternalEvent)`；`stop` 标志退出（进程生命周期，不做 join）。
- [ ] **Step 1：测试**：`WatchStatus` camelCase（`degraded`）；`watch_retarget` 无 state 不可测→以 `WatchRegistry` 单测 + `snapshot_for_path` 单测（插两个 DocFile 同 path，返回两条快照）覆盖。
- [ ] **Step 2：** FAIL（类型未定义）→ **Step 3：实现**（watcher 创建失败置 `degraded` 且不返错；`watch_subscribe` 先注册 channel 再创建 watcher；subscribe 前的 registry 目录在 `start` 时补 watch）。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:watch-runtime-and-hooks` + push。

---

## Task 3: `AppDocs::replace` + `doc_reload` 命令

**Files:** Modify `app/src-tauri/src/state.rs`（`pub fn replace(&self, id:u64, doc:DocFile)->bool`）；Modify `commands.rs`（`doc_reload` + 可测 helper `fn reload_doc(state:&AppDocs, doc_id:u64)->Result<DocInfo,IpcError>`）；Modify `lib.rs`（注册）。
**Interfaces:** `reload_doc`：`state.get(id)`→`doc.path().to_path_buf()`→`validate_open_path(&p, SIZE_LIMIT)`→`DocFile::open`→拼 `DocInfo`（同 doc_id；`mode_str(size)`）→`state.replace(id, fresh)`（false→`unknown_doc`）→返回。命令薄壳额外发一次 `IndexProgress{done:true}`。
- [ ] **Step 1：测试**：`replace_swaps_in_place`（insert 后 replace，`get` 后 `lock().size()` 为新值；未知 id false）；`reload_after_external_rewrite`（tempfile 写 3 行→insert→外部改 5 行→`reload_doc`→totalLines 5、`doc_chunk(0,5)` 文本为新内容、`disk_meta()` 匹配新文件）。
- [ ] **Step 2：** FAIL → **Step 3：实现** → **Step 4：** PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:doc-reload` + push。

---

## Task 4: 前端基础件（ipc / tabs / listview / statusbar）

**Files:** Modify `app/frontend/src/ipc.ts`、`tabs.ts`、`listview.ts`、`statusbar.ts`、`styles.css`；Tests `ipc.test.ts`、新建 `tabs.test.ts`。
**Interfaces:**
- `ipc.ts`：`ExternalEvent{path:string,kind:"changed"|"removed"}`、`WatchStatus{degraded:string|null}`、`watchSubscribe(onEvent)`、`docReload(docId,onProgress):Promise<DocInfo>`、`watchStatus()`。
- `tabs.ts`：`TabEntry.ext` span（`.ext-dot`）；`setExternalFlag(docId, on)`；`remove` 时随 el 移除。
- `listview.ts`：`scrollTop(): number`、`setScrollTop(n:number): void`（`this.root.scrollTop = Math.max(0,n)`，滚动事件自行触发 render）。
- `statusbar.ts`：`setWatch(state:"ok"|"degraded", reason?:string)`（根元素内新增 `.status-watch`；ok 隐藏，degraded 显示"外部监听已降级" + title 原因）。
- `styles.css`：`.ext-dot`（隐藏默认；`.warn` 时间距/黄色 `var(--warn)`）、`.status-watch`。
- [ ] **Step 1：测试**：`ipc` 三包装 typeof；`tabs.test.ts` 用 stub `TabView`（`isDirty:()=>false`）断言 `setExternalFlag` 加/去 `.warn` 且不影响 `refreshDirty` 的 `.visible`。
- [ ] **Step 2：** FAIL → **Step 3：实现** → **Step 4：** `npm test` PASS + build。
- [ ] **Step 5：** CHANGELOG；commit `feat:watch-frontend-primitives` + push。

---

## Task 5: `main.ts` 事件分发（干净重载 / 脏黄点 / 删除 / 降级）

**Files:** Modify `app/frontend/src/main.ts`
**Interfaces（行为）**：启动 `watchSubscribe(handleExternalEvent)`（在 `initRecovery` 前后均可，须在用户打开文档前）；`path→docId` 由 `docStates` 反查（可能多个）。
- 干净 edit：`docReload` → 存 `scroll=handle.view.scrollDOM.scrollTop`、`sel=handle.view.state.selection.main`（clamp 到新长度）→ `loadAllText` → `replaceText` → `markSaved()` → 恢复 selection/scroll → 更新 `ds.*`（encoding/eol/invalid/totalLines）→ `refreshDirty`/`syncStatus`。
- 干净 readonly：存 `scroll=list.scrollTop()` → `docReload` → `list.invalidate()`+`setView(null,false,false)` → 清 `queryHits/Cursor` → `ds` 更新 → `list.refresh()` → `setScrollTop`。
- `DocInfo.mode` 变化 → `handleClose` 静默关（干净）+ `openPath` 重开 + toast。
- 脏：`tabs.setExternalFlag(docId,true)` + `showExtBar(docId)`（`.ext-bar` 两键：重新加载/保留）；保留→隐藏条、黄点留。
- `removed`：黄点 + toast；不自动重载。
- `reloading` Map 防重入；完成后若有 pending 再跑一次。
- 降级：`status.setWatch("degraded", reason)`。
- 重载 helper `reloadDocFromDisk(docId, discardEdits:boolean)` 共用（干净自动 / 脏"重新加载"）。
- [ ] **Step 1：实现** 上述（无新单测，DOM/IPC 密集；纯逻辑已在 T1/T4）。
- [ ] **Step 2：** `npm test` 不回归 + build tsc PASS。
- [ ] **Step 3：** CHANGELOG；commit `feat:external-change-dispatch` + push。

---

## Task 6: 保存三选模态 + Save-As retarget + 外部修改提示条样式

**Files:** Modify `app/frontend/src/main.ts`（`confirmExternalConflict(detail)` DOM 模态（保留 `err.zh` 的磁盘大小/`?` 数知情细节）；`saveActive` 的 `conflict` 分支改用它；`afterSave` 不再迁移监听（由 `doc_save` 后端完成）；`showExtBar` 逻辑完善）、`styles.css`（`.ext-bar` + `.modal-3way`）。
**Interfaces：** `confirmExternalConflict(detail: string): Promise<"reload"|"overwrite"|"cancel">`（三键 + Esc/遮罩=取消；`role="dialog"`）；`reload`→`reloadDocFromDisk(docId,true)`；`overwrite`→`force:true` 保存路径；`cancel`→`setBusy(false)`；`unmappable` 保持 `ask`。
- [ ] **Step 1：实现**。
- [ ] **Step 2：** `npm test` 不回归 + build tsc PASS。
- [ ] **Step 3：** CHANGELOG；commit `feat:save-conflict-three-way` + push。

---

## Task 7: 全量四门 + 终审 + 合 main

- [ ] **Step 1：全量四门**：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build` → 全绿。
- [ ] **Step 2：GUI 手工**：只读日志 `echo >>` 追加→自动重载且滚动保持；编辑干净→外部覆盖→文本/光标近似保持；编辑脏→外部覆盖→黄点+提示条→保存三选（reload/overwrite/cancel 三支）；删除文件→提示；Save-As 后监听迁移（再外部改新路径→有事件）。
- [ ] **Step 3：** CodeReview 全分支（diff 包 + spec/plan + Review Focus），修复波 + 定向复审；`--no-ff` 合 `main`、push、更新 `docs/claude.md`（外部修改监听勾选）、删 SDD 工作区。
- [ ] **Step 4：** 合并后在 `main` 复跑 `cargo test -p lumelog-app` + `run build`；`Cargo.lock` 随 T2 提交。

---

## 备注：接缝/决策
- `ListView.root` 即滚动容器（`addEventListener("scroll")`/`scrollTop`），无需新内部结构。
- `tabs` 脏点是 `.dirty-dot` 文字 `●`；黄点用独立 `.ext-dot`（文字同样 `●` + `.warn` 颜色），互不影响。
- `doc_reload` 用 `AppDocs::replace` 原地替换（保 Arc 一致）；`reload_doc` 抽为无 `State` 的 helper 以便单测。
- `notify` 只在 `watch_subscribe` 时创建；创建失败→`degraded`，打开/保存不受影响。
- 去抖线程常驻（进程级）；不做 stop/join（单窗口应用，退出即终止）。
- 保存三选仅对 `conflict`；`unmappable` 保持两键，避免语义混淆。
