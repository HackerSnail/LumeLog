# Roadmap-3：QuickJS 脚本引擎 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 脚本管理器（列表/编辑/保存/删除）+ 控制台；对当前文档逐行流式执行 `onLine(line, ctx)`，结果原子写入新文件；有内存/栈/时间/取消/输出上限，无 fs/net/require。

**Architecture:** `app/src-tauri/src/script.rs` 用 `rquickjs` 0.14 执行一次源码取 `onLine`，`DocLines` 逐行调用、`BufWriter`→tmp→rename；命令 `script_list/read/write/delete/run/cancel`。前端 `script-ui.ts` 纯 DOM 模态（回调注入 IPC），`main.ts` 接线命令面板/工具栏/快捷键互斥。

**Tech Stack:** `rquickjs = "0.14"`（QuickJS C 经 cc 编译）；workspace `rust-version` 1.79 → 1.87；TypeScript。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

**Spec:** `docs/specs/2026-10-25-roadmap-script-design.md`。分支基 `main`（`ba96033`）→ `feat/roadmap-script`。

## Global Constraints

- 脚本名 `^[A-Za-z0-9_-]{1,64}\.js$`；目录 `$APP_CONFIG/LumeLog/scripts`；读写删只用该目录内文件。
- JS 契约：`onLine(line, ctx)` 返回 `string` 保留/改写、`null|undefined` 丢弃，其它→带行号错误。
- ctx `{lineNumber(1 起), index(0 起), path}`；无宿主 fs/net/require/process。
- 限制：内存 64 MiB、栈 1 MiB、单次 300s、控制台 200 条×4 KiB、输出 4 GiB；超限即停并保留前缀。
- 输出 UTF-8 + `\n`；目标 == 源文档路径 → 拒绝；tmp→rename，失败删 tmp。
- 取消：行间 + JS interrupt 双检查；`cancelled` 与“超时/错误”分开上报；`on_done` 必发。
- 前端：脚本模态与 keymap 模态互斥；`scriptModalOpen` 让全局快捷键短路；无文档可管理/编辑，但运行禁用。
- 简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；push；`--no-ff` 合 main。

## Review Focus

1. **沙箱**：无 fs/net/require；内存/栈/时间中断确实生效（死循环、`new Array(1e9)`）；console 有界；错误信息不泄内部路径（除 ctx.path 用户已知）。
2. **契约与流式**：三态返回；非字符串报错带行号；`DocLines` 分块不整文件；取消保留前缀；`cancelled`/超时/错误区分。
3. **写盘**：tmp→rename；目标==源拒绝；输出 >4 GiB；编码固定 UTF-8/LF；失败无半文件。
4. **脚本目录**：名字校验防穿越；list 缺目录/坏文件；write/delete 幂等与原子。
5. **前端**：无文档/运行中禁用；取消；控制台追加与截断提示；未保存切换/关闭确认；两模态互斥。
6. **MSRV**：workspace 1.87 与 rquickjs 0.14 一致；Windows 下 QuickJS C 构建通过。

---

## Task 1: MSRV + `rquickjs` + 脚本存储/工具

**Files:** Modify `Cargo.toml`（`rust-version = "1.87"`）、`app/src-tauri/Cargo.toml`（`rquickjs = "0.14"`）；Create `app/src-tauri/src/script.rs`；Modify `app/src-tauri/src/lib.rs`（`mod script;`）；`Cargo.lock`
**Interfaces:** `pub(crate) fn validate_name(name:&str)->Result<(),IpcError>`；`fn scripts_dir(app:&AppHandle)->Result<PathBuf,IpcError>`；`list_scripts/read_script/write_script/delete_script(app,...)`；控制台累加器 `ConsoleBuf::push(level,text)->bool`（上限/截断标记）。
- [ ] **Step 1：** 改 MSRV/依赖，`cargo check -p lumelog-app`（确认 QuickJS C 构建可用）。
- [ ] **Step 2：测试**：`validate_name`（`a.js` 合法；`a/b.js`、`../a.js`、`a.txt`、空、65 字符拒绝）；`ConsoleBuf`（格式化：多参数空格连接、对象 JSON、非可序列化 fallback、单条截 4096、200 条后 `truncated` 且不再增长）；`write/read/delete` 临时目录 roundtrip + 原子性（写入后无 `.tmp*`）。
- [ ] **Step 3：** FAIL→实现→PASS + clippy。
- [ ] **Step 4：** CHANGELOG；commit `feat:script-store` + push。

---

## Task 2: `run_script` 运行器（QuickJS）

**Files:** Modify `app/src-tauri/src/script.rs`
**Interfaces:** `pub fn run_script(doc: Arc<Mutex<DocFile>>, code:&str, target:&Path, on_console:&mut dyn FnMut(ScriptConsole), on_progress:&mut dyn FnMut(ScriptProgress), cancel:&dyn Fn()->bool) -> ScriptStats`；`ScriptConsole{level,text}`/`ScriptProgress{scanned,written}`/`ScriptStats{scanned,written,dropped,cancelled,truncated_console,error}`。
- [ ] **Step 1：测试**（临时文档 `"a\n\nb\n"`）：`keep_replace_drop`（返回 line / 大写 / null → 输出 `A\nB\n`）、`ctx_line_numbers`（脚本把 `ctx.lineNumber` 写入输出，断言 1/2/3）、`js_throw_stops_with_prefix`（第 2 行抛错 → 输出仅第 1 行、`error` 含行号）、`missing_on_line_errors`、`utf8_lf_bytes`、`console_capture`（log/warn/error 各一条 + 截断标记）、`output_paths_are_atomic`（成功后无 tmp）。
- [ ] **Step 2：** FAIL→实现（Runtime 限制 + interrupt 截止；`Context::full`；注入 console；eval；取 `onLine`；DocLines 循环；BufWriter+tmp→rename）。
- [ ] **Step 3：** PASS + 全 workspace 测试不回归 + clippy 净。
- [ ] **Step 4：** CHANGELOG；commit `feat:script-runner` + push。

---

## Task 3: 命令 + DTO + 注册

**Files:** Modify `app/src-tauri/src/commands.rs`、`dto.rs`、`lib.rs`
**Interfaces:**
- DTO（camelCase）：`ScriptEntry{name,sizeBytes,modifiedMs}`、`ScriptConsole{level,text}`、`ScriptProgress{scanned,written}`、`ScriptStatsJson{scanned,written,dropped,cancelled,truncatedConsole,error}`。
- `script_list/script_read/script_write/script_delete`（同步）；`script_run(state, app, doc_id:u64, code:String, target_path:String, on_console:Channel<ScriptConsole>, on_progress:Channel<ScriptProgress>, on_done:Channel<ScriptStatsJson>) -> Result<u64,IpcError>`（prologue：doc 存在、目标≠源、目录可建；`begin_search`+spawn_blocking+`end_search`）；`script_cancel(req_id)`。
- [ ] **Step 1：测试**：`script_stats_camel_case`；`script_write_rejects_bad_name`（走 helper）；目标==源拒绝（纯 helper `same_path`）。
- [ ] **Step 2：** FAIL→实现+注册→PASS + clippy。
- [ ] **Step 3：** CHANGELOG；commit `feat:script-commands` + push。

---

## Task 4: 前端 `ipc.ts`

**Files:** Modify `app/frontend/src/ipc.ts`、`ipc.test.ts`
**Interfaces:** `ScriptEntry/ScriptConsole/ScriptProgress/ScriptStats`；`scriptList/scriptRead/scriptWrite/scriptDelete/scriptRun/scriptCancel`（`scriptRun` 双 Channel：console/progress，done 在 `onDone`）。
- [ ] **Step 1：** typeof 测试（6 个）。
- [ ] **Step 2：** FAIL→实现→PASS + build。
- [ ] **Step 3：** CHANGELOG；commit `feat:ipc-script` + push。

---

## Task 5: `script-ui.ts` 模态 + jsdom 测试

**Files:** Create `app/frontend/src/script-ui.ts`、`script-ui.test.ts`；Modify `styles.css`
**Interfaces:** `openScriptModal(opts: { hasDocument:()=>boolean; defaultOutputBase:()=>string; list:()=>Promise<ScriptEntry[]>; read:(n:string)=>Promise<string>; write:(n:string,t:string)=>Promise<void>; remove:(n:string)=>Promise<void>; chooseTarget:(base:string)=>Promise<string|null>; run:(a:{code:string;targetPath:string;onConsole:(m:ScriptConsole)=>void;onProgress:(p:ScriptProgress)=>void;onDone:(s:ScriptStats)=>void})=>Promise<number>; cancel:(id:number)=>Promise<void>; confirm:(msg:string)=>Promise<boolean>; onClose:()=>void })`
- [ ] **Step 1：测试**：列表渲染；新建→模板+立即写盘；保存调用 `write`；运行禁用（`hasDocument=false`）/启用并传 code+target；`onConsole` 追加带级别 DOM 行；`onDone` 状态行。
- [ ] **Step 2：** FAIL→实现（左列表/新建/删除；右 textarea+Tab 两空格；底部 console/状态/运行/取消/关闭；未保存切换/关闭 `confirm`；运行中禁用编辑；`inert` 背景）。
- [ ] **Step 3：** PASS + `styles.css`（`.script-*`）→ build。
- [ ] **Step 4：** CHANGELOG；commit `feat:script-ui` + push。

---

## Task 6: `main.ts` 接线

**Files:** Modify `app/frontend/src/main.ts`
**Interfaces（行为）：**
- `let scriptModalOpen = false;` `registerShortcuts` 开头 `if (keymapModalOpen || scriptModalOpen) return;`；两模态互斥（open 前检查）。
- 顶部工具栏“脚本”按钮 + 面板命令 `{ id:"script", title:"脚本…", run: () => openScriptManager() }`（不加入 `ACTION_DEFS`）。
- `openScriptManager()`：`scriptModalOpen=true`；`openScriptModal({ hasDocument: () => tabs.active()!=null, defaultOutputBase: () => basenameOf(activeDocPath) 或 "script", list/read/write/remove/run/cancel 用 ipc 包装, confirm: (m)=>ask(m,{title:"脚本"}), chooseTarget: (base)=>save({defaultPath:`${base}.out.txt`, filters:[{name:"文本",extensions:["txt"]}]}).then(v=>typeof v==="string"?v:null), onClose: ()=>{scriptModalOpen=false;} })`。
- `run` 包装：解析出 `docId`（当前活动文档）；无则 throw；调用 `scriptRun` 并返回 reqId；`onConsole/onProgress/onDone` 原样转发。
- [ ] **Step 1：实现**；**Step 2：** `npm test` 不回归 + build；**Step 3：** CHANGELOG；commit `feat:script-wiring` + push。

---

## Task 7: 四门 + 终审 + 合 main

- [ ] **Step 1：** 四门全绿（含 `cargo test --workspace` 的 script 测试）。
- [ ] **Step 2：GUI 手工**：脚本管理器新建/保存/删除；运行“丢空行+大写+行号前缀”脚本→输出正确；语法/运行错误显示且不崩；运行中取消保留前缀；死循环 300s 内被中断；无文档时运行禁用；keymap 模态与脚本模态不叠加。
- [ ] **Step 3：** CodeReview 全分支 + 修复波 + 定向复审；`--no-ff` 合 main、push、`docs/claude.md` 勾选 Roadmap 脚本、删 SDD 工作区。
- [ ] **Step 4：** 合并后 `main` 复跑 `cargo test --workspace` + `run build`。

---

## 备注：接缝/决策
- `script.rs` 与 `commands.rs` 同 crate：复用 `DocLines`（`pub(crate)`）与 `begin_search`；不把 JS 引擎放进 `lumelog-core`。
- `run_script` 内部完成 tmp→rename；命令层只校验目标≠源与目录可写，不重复写盘。
- interrupt handler 用 `Rc/RefCell` 或 `Arc<AtomicBool>` + `Instant`；`cancel()` 为 `&dyn Fn`，handler 里捕获克隆。
- jsdom 测试用注入回调，不 mock ipc；`script-ui` 不 import `ipc.ts`。
- 新建脚本模板：`function onLine(line, ctx) {\n  return line;\n}`。
- MSRV 变更写入 CHANGELOG 与 `docs/claude.md`（Roadmap 脚本条目）。
