# M3 子项目 B1：崩溃恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 编辑模式脏文档停输 2s 写影子文件（全量文本 + 编码/行尾元数据）；异常退出后启动以**单列表横幅**逐项"恢复/丢弃"；恢复后仍脏、保存才落盘；保存/丢弃后删影子；只读不产影子。

**Architecture:** 后端 `app/src-tauri/src/recovery.rs` 纯存储层（首行 JSON meta + `\n` + 原文；`doc_key=FNV-1a64(path)`；tmp→rename）+ 4 命令（**全以 `path` 为键**，后端推 key）。前端 `recovery.ts` 提供 `Debouncer`（2s 按 docId 合并）+ `humanSize`/`formatSavedAt`；`main.ts` 在 `onDirty` 排影、`afterSave`/`handleClose`/`changeEncoding` 清理、启动 `recoveryList()` 显示 banner 并恢复/丢弃。

**Tech Stack:** Rust（Tauri v2 `app.path().app_config_dir()`，零新依赖、FNV 手写、`serde_json` 已在）+ TypeScript 原生 DOM。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm --prefix app/frontend test` / `run build`。

**Spec:** `docs/specs/2026-10-25-m3-crash-recovery-design.md`。承接 M1–M3（`doc_save` 原子写、`EditorHandle.getText()`（已存在）、`DocState{path,encoding,eol,mode}`、`afterSave`/`handleClose`/`changeEncoding`）。

## Global Constraints

- 简体中文 UI；DTO/磁盘 JSON `#[serde(rename_all="camelCase")]`。
- 影子目录 `app_config_dir()/LumeLog/recovery/`；文件 `<doc_key>.lumelog` = 首行 JSON meta + `\n` + 原始 UTF-8 文本；`write_shadow` 用 `.tmp`→rename（不强制 fsync）。
- 命令以 `path` 为键（前端不算 FNV）；`recovery_discard` 幂等；目录缺失 list→空。
- 只读文档不写影子；恢复后文档必须**仍脏**；恢复即丢弃旧影子。
- 逐任务 TDD；四门绿再提交；commit 前追加 `CHANGELOG.md`；commit 消息无空格；commit 后 push；分支 `--no-ff` 合 `main`。
- 分支基：从 `main`（当前 `7e88a5f`，+ 本计划提交）新建 `feat/m3-crash-recovery`。

## Review Focus

1. **影子往返完整性**：meta 四字段 + 文本逐字符（换行/中文/emoji）；首行 JSON 不含裸换行（serde 转义）→ 首个 `\n` 就是分隔符。
2. **`doc_key` 稳定性**：FNV-1a64 标准向量（空串 `0xcbf29ce484222325`、"a" `0xaf63dc4c8601ec8c`）；同 path 同 key；不同 path 不同。
3. **list 健壮性**：按 `savedAtMs` 降序、损坏影子跳过、目录不存在→空。
4. **debounce 语义**：连发只写一次、`cancel` 后不再触发、按 docId 隔离；只脏编辑文档排影（只读不排）。
5. **清理时机**：`afterSave`（含 Save-As 前旧 path + 新 path）、`handleClose`、`changeEncoding` 丢弃重载后都清影 + 清定时器。
6. **恢复正确性**：原文件缺失→提示不删条目；恢复后 `docStates.encoding/eol` 取影子元数据且文档脏；恢复后旧影子被删。

---

## Task 1: 后端 `recovery` 存储层

**Files:**
- Create: `app/src-tauri/src/recovery.rs`（类型 + 纯函数 + `#[cfg(test)] mod tests`）
- Modify: `app/src-tauri/src/lib.rs`（`mod recovery;`）
- Test: `recovery.rs`

**Interfaces:**
- Produces:
  - `pub struct RecoveryMeta { pub path:String, pub encoding:String, pub eol:String, pub saved_at_ms:u64 }`（camelCase）
  - `pub struct RecoveryEntry { pub path:String, pub encoding:String, pub eol:String, pub saved_at_ms:u64, pub size_bytes:u64 }`（camelCase）
  - `pub struct RecoveryContent { pub path:String, pub encoding:String, pub eol:String, pub text:String }`（camelCase）
  - `pub fn fnv1a64(bytes:&[u8])->u64`；`pub fn doc_key(path:&str)->String`（`{:016x}`）
  - `pub fn shadow_path(dir:&Path, path:&str)->PathBuf`
  - `pub fn write_shadow(dir:&Path, meta:&RecoveryMeta, text:&str)->io::Result<()>`（`create_dir_all` + tmp→rename）
  - `pub fn read_shadow(dir:&Path, path:&str)->io::Result<(RecoveryMeta, String)>`（首个 `\n` 切分；坏 meta→`InvalidData`）
  - `pub fn list_shadows(dir:&Path)->Vec<RecoveryEntry>`（缺失目录→空；坏文件跳过；`saved_at_ms` 降序）
  - `pub fn discard_shadow(dir:&Path, path:&str)`（幂等，忽略 NotFound）
- 模块顶部 `#![allow(dead_code)] // consumed by commands (Task 2)`，Task 2 剥。

- [ ] **Step 1：写失败测试**（`tempfile::tempdir()`）：`fnv_vectors`（空串/a）；`doc_key_stable_distinct`；`roundtrip_meta_and_text`（文本含 `\n` 中文 emoji，断言逐字符 + 首行 JSON）；`list_sorted_and_skips_bad`（写两条目 + 手放一个坏文件，断言降序 2 条且坏跳过）；`list_missing_dir_empty`；`discard_idempotent`。
- [ ] **Step 2：** `cargo test -p lumelog-app recovery::` → FAIL。
- [ ] **Step 3：实现** 上述类型/函数（meta 序列化用 `serde_json::to_string`；读取 `fs::read` 后按首个 `b'\n'` 切分，meta 反序列化失败→InvalidData；`list_shadows` 读目录、逐个 `read_shadow` 失败则 skip、读文件大小填 `size_bytes`、按 `saved_at_ms` 降序）。
- [ ] **Step 4：** `cargo test -p lumelog-app recovery::` PASS + `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:recovery-shadow-store` + push。

---

## Task 2: 四命令 + 注册

**Files:**
- Modify: `app/src-tauri/src/commands.rs`（`recovery_dir(app)->Result<PathBuf,IpcError>` + `recovery_save/list/load/discard`）
- Modify: `app/src-tauri/src/lib.rs`（注册 4 命令）
- Modify: `app/src-tauri/src/recovery.rs`（剥模块 `#![allow(dead_code)]`）
- Test: `commands.rs`（`recovery_cmd_tests`：DTO camelCase、`recovery_dir` 拼接）

**Interfaces:**
- `fn recovery_dir(app:&AppHandle) -> Result<PathBuf,IpcError>`：`app.path().app_config_dir()`→join `"LumeLog"`/`"recovery"`，`io` 错误映射。
- `#[tauri::command] recovery_save(app, path:String, encoding:String, eol:String, text:String) -> Result<(),IpcError>`
- `#[tauri::command] recovery_list(app) -> Result<Vec<recovery::RecoveryEntry>,IpcError>`
- `#[tauri::command] recovery_load(app, path:String) -> Result<recovery::RecoveryContent,IpcError>`
- `#[tauri::command] recovery_discard(app, path:String) -> Result<(),IpcError>`
- 均同步命令（无重活，文本大时也仅一次写/读；不 spawn）。

- [ ] **Step 1：写失败测试**：`recovery_entry_camel_case`（`serde_json::to_value(RecoveryEntry)` 键 `savedAtMs/sizeBytes` + `path/encoding/eol`）；`recovery_content_camel_case`。
- [ ] **Step 2：** `cargo test -p lumelog-app recovery_cmd` → FAIL。
- [ ] **Step 3：实现** `recovery_dir` + 4 命令（`recovery_save` 构造 `RecoveryMeta` 取当前 ms；`list` 调 `list_shadows`；`load` 组装 `RecoveryContent`；`discard` 调 `discard_shadow`）；`lib.rs` 注册；剥 recovery 模块 allow。
- [ ] **Step 4：** `cargo test -p lumelog-app` PASS + `cargo clippy --workspace --all-targets -- -D warnings` 净。
- [ ] **Step 5：** CHANGELOG；commit `feat:recovery-commands` + push。

---

## Task 3: 前端 `recovery.ts`（Debouncer + 格式化）

**Files:** Create `app/frontend/src/recovery.ts`；Test `app/frontend/src/recovery.test.ts`
**Interfaces:** `export class Debouncer { constructor(delayMs:number, fire:(docId:number)=>void); schedule(docId:number):void; cancel(docId:number):void; cancelAll():void; }`；`export function humanSize(bytes:number):string`（`B/KB/MB/GB`，1 位小数，整数去尾零）；`export function formatSavedAt(ms:number):string`（本地 `YYYY-MM-DD HH:mm:ss`）。
- [ ] **Step 1：写失败测试**（`vi.useFakeTimers()`）：`debounce_coalesces`（schedule 3 次→advance 1999 不触发→advance 1 触发 1 次，id 最后一次）；`debounce_cancel`（cancel 后 advance 不触发）；`debounce_isolates_ids`；`humanSize_cases`（0→`0 B`、1023→`1023 B`、1024→`1 KB`、1.5MB）；`formatSavedAt_shape`（正则 `^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$`）。
- [ ] **Step 2：** `npm test` → FAIL。
- [ ] **Step 3：实现**（`Map<number, ReturnType<typeof setTimeout>>`；schedule 先 cancel 同 id 再 setTimeout；`cancel` clear+delete）。
- [ ] **Step 4：** `npm test` PASS + `run build` tsc PASS。
- [ ] **Step 5：** CHANGELOG；commit `feat:recovery-debouncer` + push。

---

## Task 4: `ipc.ts` 桥

**Files:** Modify `app/frontend/src/ipc.ts`；Test `ipc.test.ts`
**Interfaces:** `RecoveryEntry{path,encoding,eol,savedAtMs,sizeBytes}`、`RecoveryContent{path,encoding,eol,text}`；`recoverySave(args:{path,encoding,eol,text})`、`recoveryList():Promise<RecoveryEntry[]>`、`recoveryLoad(path:string):Promise<RecoveryContent>`、`recoveryDiscard(path:string):Promise<void>`。
- [ ] **Step 1：写失败测试**：四函数 typeof；`recoverySave` mock invoke 断言 payload 键（`path/encoding/eol/text`）；`recoveryLoad/recoveryDiscard` 传 `{path}`。
- [ ] **Step 2：** FAIL → **Step 3：实现** → **Step 4：** PASS + build → **Step 5：** CHANGELOG；commit `feat:ipc-recovery-bridge` + push。

---

## Task 5: `main.ts` 排影 + 清理

**Files:** Modify `app/frontend/src/main.ts`
**Interfaces:** 复用 `editors`/`docStates`/`afterSave`/`handleClose`/`changeEncoding`；新增 `const recoveryDebounce = new Debouncer(2000, fireRecovery)`；`function scheduleRecovery(docId:number)`（handle 脏才 schedule）；`function cancelRecovery(docId:number, path:string)`（cancel + `recoveryDiscard(path).catch`）。
- [ ] **Step 1：实现**：
  - `mountDoc` 的 `onDirty` 回调追加 `scheduleRecovery(info.docId)`（在 `tabs.refreshDirty` 之后）。
  - `fireRecovery(docId)`：取 `ds=docStates.get`、`h=editors.get`；缺或 `!h.isDirty()` 则 return；`recoverySave({path:ds.path,encoding:ds.encoding,eol:ds.eol,text:h.getText()}).catch(() => {/* best-effort，状态栏一次提示可省 */})`。
  - `saveActive`：Save-As 前记 `const prevPath = ds.path;`；两处 `afterSave(...)` 追加实参 `prevPath`；`afterSave(..., target, prevPath)` 内 `cancelRecovery(docId, prevPath)` + `cancelRecovery(docId, target)`（重复无害）。
  - `handleClose`：在 `docStates.delete` 前，若 `ds` 存在 `cancelRecovery(docId, ds.path)`。
  - `changeEncoding`：成功路径（`handle` 存在、重载后）调 `cancelRecovery(docId, ds.path)`。
- [ ] **Step 2：** `npm test` 不回归 + `run build` tsc PASS（`prevPath` 两处调用点必须同步，否则 tsc 报错）。
- [ ] **Step 3：** CHANGELOG；commit `feat:recovery-schedule-and-cleanup` + push。

---

## Task 6: 启动横幅 + 恢复/丢弃

**Files:** Modify `app/frontend/src/main.ts`（`checkRecovery()` + banner DOM + `restoreRecovery`）、`app/frontend/src/styles.css`（`.recovery-banner`）
**Interfaces:** 启动在 `appPing` 之后调 `void checkRecovery()`；`recoveryList()` 非空→渲染 `.recovery-banner`（标题 + 列表：`basenameOf(path)` 尾段、`formatSavedAt(savedAtMs)`、`humanSize(sizeBytes)`、每行"恢复"/"丢弃" + "全部丢弃" + 关闭）。恢复：`docOpen(entry.path)`→`recoveryLoad(entry.path)`→`editors.get(docId)!.replaceText(content.text)`→`ds.encoding=content.encoding; ds.eol=content.eol`；`recoveryDiscard(entry.path)`；移除行；空则移除 banner。打开失败（IpcError）→`toastMsg("原文件已缺失，无法恢复：…")` 且**不删条目**。
- [ ] **Step 1：实现** `checkRecovery`/`buildRecoveryBanner`/`restoreOne`/`discardOne`/`discardAll` + CSS（固定右下/顶部条，`role="status"`、可滚动列表、按钮复用 `.btn` 风格）。
- [ ] **Step 2：** `npm test` 不回归 + `run build` tsc PASS。
- [ ] **Step 3：** CHANGELOG；commit `feat:recovery-banner-restore` + push。

---

## Task 7: 全量四门 + 终审 + 合 main

- [ ] **Step 1：全量四门**：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm --prefix app/frontend test` / `npm --prefix app/frontend run build` → 全绿。
- [ ] **Step 2：GUI 手工清单**（`tauri dev`）：脏文档停 2s→影子出现；`kill -9` 重启→banner→恢复→文本回填仍脏→保存后影子消失；丢弃→影子删；只读不产影子；坏影子被跳过；原文件缺失提示且条目保留。
- [ ] **Step 3：** 派 CodeReview 审整条 `feat/m3-crash-recovery`（全分支 diff 包 + spec/plan + 约束/Review Focus），修复波 + 定向复审；清干净后 `--no-ff` 合 `main`、push、更新 `docs/claude.md`（"崩溃恢复"勾选）、删本计划 SDD 工作区。
- [ ] **Step 4：** 合并后在 `main` 复跑 `cargo test -p lumelog-app` + `run build` 复验；`Cargo.lock` 无新依赖（不预期变动）。

---

## 备注：接缝/决策
- `EditorHandle.getText()` 已存在（`app/frontend/src/editor.ts:35`），无需改 editor。
- 命令以 `path` 为键（spec 已按此修订）：前端不实现 FNV；`afterSave` 新增 `prevPath` 参数（Save-As 前旧 path 也要清影）。
- 影子写失败 best-effort：不弹模态、不中断编辑；下一次停输重试。
- `recovery_enabled`/`config.toml` 不在本增量（spec Ruling，v1 恒开）。
- 恢复后用影子 `encoding/eol` 覆盖 `DocState`，使后续保存按原编码/行尾落盘；`replaceText` 触发 `onDirty` 保证"恢复后仍脏"。
