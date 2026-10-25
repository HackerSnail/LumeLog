# Roadmap-1：tail 跟随 + 滚动截断 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 只读日志外部追加时增量扩展索引并自动滚底（可开关、上滚暂停）；截断/轮转全量重载并保持跟随；编辑模式不跟随。

**Architecture:** core：`LineIndex::extend` + `DocFile::try_append`（头签名/尺寸判轮转，成功即刷新 `(size,mtime)` 快照）。app：`doc_tail_append` 命令。前端：`ListView` 滚动辅助 + `isNearBottom`；状态栏“跟随尾部”开关；`main.ts` 对只读 `changed` 先增量、`rotated` 再 `doc_reload`。

**Tech Stack:** Rust（无新依赖）+ TypeScript。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm --prefix app/frontend test` / `run build`。

**Spec:** `docs/specs/2026-10-25-roadmap-tail-follow-design.md`。承接：B2 外部监听/自动重载、`ListView{scrollTop,setScrollTop}`、`Statusbar`。

## Global Constraints

- core 不重建全文：`extend` 只扫追加字节的 `0x0A`；越界（>u32::MAX）→`CoreError::TooLarge`。
- `try_append` 成功必须刷新 `size/mtime`（外加重算头签名），否则 B2 watcher 会重复报同一增长。
- 轮转判定：`new_size < old_size` OR 新头 `min(512,new_size)` 的 FNV-1a64 != `head_sig`。
- 只读 + 手动开关；follow 状态按 docId 记忆；上滚暂停并显示“回到底部”；编辑模式无开关。
- 追加提示 ≥1s 节流；`rotated` 走现有 `doc_reload`。
- 简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；commit 消息无空格；commit 后 push；分支 `--no-ff` 合 `main`。
- 分支基：从 `main`（当前 `6121a75`）新建 `feat/roadmap-tail-follow`。

## Review Focus

1. **索引增量正确性**：旧文件尾有/无 LF 两种起点、空追加、多 LF 跨块、4GB 越界；`chunk_at` 对新行返回正确文本。
2. **轮转/截断识别**：缩小→Rotated；同大小改写头部→Rotated；正常追加→Appended；无变化→NoChange。
3. **快照刷新**：`try_append` 后 `disk_meta` 等于磁盘，watcher 不再重复触发。
4. **follow 语义**：开→滚底；上滚→暂停+提示；回底/点提示→恢复；关→不动；切标签恢复各自状态；编辑文档无按钮。
5. **变更分流**：只读 `changed` 走增量，`rotated` 走全量；追加不重置 `scrollTop`（未跟随时）；行数与 `indexed` 同步更新。
6. **缓存与内存**：旧行缓存不清、新行按需读取（虚拟列表窗口不变）。

---

## Task 1: core `LineIndex::extend`

**Files:** Modify `crates/lumelog-core/src/doc/line_index.rs`；Test 同文件 `#[cfg(test)]`
**Interfaces:** `pub fn extend(&mut self, appended:&[u8], base_offset:u64) -> Result<(), CoreError>`（每个 `0x0A` 推 `base_offset+i+1`；`u32::MAX` 越界→`TooLarge`；`file_size = base_offset + appended.len()`）
- [ ] **Step 1：测试**：`extend_after_trailing_lf`（`"a\nb\n"` 后追加 `"c\n"` → 行数 3→4，`line_range(3)` 起点正确）；`extend_after_no_trailing_lf`（`"a\nb"` 追加 `"\nc"` → 3 行，末行 `"c"`）；`extend_empty_noop`；`extend_multiple_lf`（一次 `"x\ny\n"`）；`extend_overflow_rejected`（`base_offset=u32::MAX` 且含 LF → TooLarge）。
- [ ] **Step 2：** `cargo test -p lumelog-core line_index::tests::extend` → FAIL。
- [ ] **Step 3：实现**（u64 算偏移，`u32::try_from` 失败→TooLarge）。
- [ ] **Step 4：** PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:line-index-extend` + push。

---

## Task 2: core `DocFile::try_append` + 头签名

**Files:** Modify `crates/lumelog-core/src/doc/file.rs`；Test 同文件
**Interfaces:**
- 私有 `head_sig: u64`（FNV-1a64 over `head[..min(512,len)]`，`open` 与 `reload_after_save` 时设置；`set_encoding` 不动）
- `pub enum AppendOutcome { NoChange, Appended { lines_added:u64, total_lines:u64 }, Rotated }`
- `pub fn try_append(&mut self) -> Result<AppendOutcome, CoreError>`（§2 逻辑；`stat` 失败→`Io`，文件不存在也算 Io，由前端按 removed 处理）
- [ ] **Step 1：测试**：`try_append_grows_and_chunks`（temp `"a\n"`→追加 `"b\nc"`：`Appended{lines_added:2,total:3}`、`chunk_at(2,1)` 得 `"c"`、`disk_meta().0`==磁盘）；`try_append_no_change`；`try_append_truncated_is_rotated`（先写长再截短）；`try_append_head_change_is_rotated`（等大小改写头 512B 内字节）；`try_append_after_trailing_lf`（见 §2）。
- [ ] **Step 2：** FAIL → **Step 3：实现** → **Step 4：** PASS + clippy 净 + `cargo test -p lumelog-core` 不回归。
- [ ] **Step 5：** CHANGELOG；commit `feat:doc-try-append` + push。

---

## Task 3: `doc_tail_append` 命令 + DTO

**Files:** Modify `app/src-tauri/src/dto.rs`（`TailAppendJson` camelCase）；`commands.rs`（可测 `fn tail_append(state:&AppDocs, doc_id:u64) -> Result<TailAppendJson,IpcError>` + 命令薄壳）；`lib.rs`（注册）
**Interfaces:** `tail_append`：`get(doc_id)`→锁→`try_append`→映射：`Rotated`→`{rotated:true,lines_added:0,total_lines:当前,size:当前}`；`NoChange`→`{rotated:false,0,total,size}`；`Appended`→对应值。
- [ ] **Step 1：测试**：`tail_append_grows`（temp 追加后 linesAdded/total 正确、`disk_meta` 刷新）；`tail_append_unknown_doc`（`unknown_doc`）。
- [ ] **Step 2：** FAIL → **Step 3：实现** → **Step 4：** `cargo test -p lumelog-app` PASS + clippy 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:doc-tail-append` + push。

---

## Task 4: 前端 `ipc.ts` + `listview` 滚动辅助

**Files:** Modify `app/frontend/src/ipc.ts`、`listview.ts`；Test `ipc.test.ts`、`listview.test.ts`
**Interfaces:**
- `ipc.ts`：`TailAppend{rotated:boolean;linesAdded:number;totalLines:number;size:number}`、`docTailAppend(docId):Promise<TailAppend>`。
- `listview.ts`：`export function isNearBottom(scrollTop:number, maxScroll:number, threshold=48): boolean`；`ListView` 方法 `scrollToBottom()`、`maxScrollTop()`；`ListViewOpts` 新增 `onScrollTop?: (top:number, maxScroll:number)=>void`，在现有滚动处理里调用。
- [ ] **Step 1：测试**：`isNearBottom`（0/max/阈值内/外/负 max 夹 0）；`ipc` 包装 typeof。
- [ ] **Step 2：** FAIL → **Step 3：实现** → **Step 4：** `npm test` PASS + build。
- [ ] **Step 5：** CHANGELOG；commit `feat:listview-scroll-helpers` + push。

---

## Task 5: 状态栏“跟随尾部”

**Files:** Modify `app/frontend/src/statusbar.ts`、`styles.css`；Test `statusbar.test.ts`（jsdom）
**Interfaces:** 回调 `onFollow?: (on:boolean)=>void`；`setFollowAvailable(on:boolean)`（按钮显隐）；`setFollow(on:boolean)`（`.active`）；按钮 `.status-follow` 文本“跟随尾部”，点击切换。
- [ ] **Step 1：测试**：默认隐藏；`setFollowAvailable(true)` 显示；`setFollow(true)` 加 `.active`；点击回调收到 `true`/`false`。
- [ ] **Step 2：** FAIL → **Step 3：实现** + `.status-follow`/`.active` 样式 → **Step 4：** PASS + build。
- [ ] **Step 5：** CHANGELOG；commit `feat:statusbar-follow-toggle` + push。

---

## Task 6: `main.ts` 跟随状态 + 只读 `changed` 分流

**Files:** Modify `app/frontend/src/main.ts`、`styles.css`（`.follow-resume`）
**Interfaces（行为）：**
- `const follow = new Map<number,{on:boolean;paused:boolean}>()`；`mountList`：`status.setFollowAvailable(true)`、`follow.set(docId,{on:false,paused:false})`，并传 `onScrollTop` 回调；`mountDoc`：`setFollowAvailable(false)`；切标签（`syncStatus` 处）按目标 doc 恢复可用性/状态。
- `onFollow(on)`：记录；`on` 时 `list.scrollToBottom()` + `paused=false`；`status.setFollow(on)`。
- 滚动回调：只读且 `follow.on` 时，`!isNearBottom` → `paused=true` + 显示 `.follow-resume`（点击→滚底+恢复+隐藏）；回底→`paused=false` + 隐藏。
- `handleExternalEvent` 对 `lists.has(docId)` 的 `changed` → `handleReadonlyChange(docId,path)`：
  1. `await docTailAppend(docId)`；
  2. `rotated` → `reloadDocFromDisk`（完成后若 follow.on → 滚底）；
  3. `Appended` → `ds.totalLines=r.totalLines`、`indexed.set`、`list.refresh()`；`follow.on&&!paused` → `scrollToBottom()`；`linesAdded>0` 且距上次提示 ≥1s → `toastMsg(\`已追加 N 行\`)`；`NoChange` 忽略；
  4. 失败（`io`）→ `reportError` 一次并停跟随（`follow.on=false`、`setFollow(false)`）。
- `reloadDocFromDisk` 只读分支：重载完成后若 follow.on → `scrollToBottom()`，否则保持 `scrollTop`。
- [ ] **Step 1：实现**；**Step 2：** `npm test` 不回归 + build tsc PASS；**Step 3：** CHANGELOG；commit `feat:tail-follow-wiring` + push。

---

## Task 7: 全量四门 + 终审 + 合 main

- [ ] **Step 1：** `cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build` → 全绿。
- [ ] **Step 2：GUI 手工**：只读日志开跟随→`echo >>` 追加（尾无/有换行）→滚底、行数递增、无整页闪；上滚→暂停+“回到底部”；回底恢复；`> file` 截断、`mv` 轮转→重载并继续跟随；编辑文档无开关；关跟随→追加不滚底。
- [ ] **Step 3：** CodeReview 全分支 + 修复波 + 定向复审；`--no-ff` 合 `main`、push、更新 `docs/claude.md`（Roadmap tail 勾选/新增行）、删 SDD 工作区。
- [ ] **Step 4：** 合并后在 `main` 复跑 `cargo test --workspace` + `run build`。

---

## 备注：接缝/决策
- `try_append` 的 `Rotated` 不修改索引；前端调用现有 `doc_reload` 重建（`AppDocs::replace` 原地）。
- `head_sig` 是私有字段；`reload_after_save` 必须同步更新（否则自家保存后首轮 append 误判轮转）。
- `ListView` 的滚动回调必须从现有 `onScroll` 路径调用（不要另加 scroll 监听，避免双触发）。
- follow 状态按 docId 记忆，标签切换时同步按钮；关闭文档时删除条目。
- 追加提示节流用模块级 `lastTailToastAt`/`tailAddedPending`。
