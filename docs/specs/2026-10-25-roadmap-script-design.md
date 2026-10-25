# Roadmap-3：QuickJS 脚本引擎（逐行变换）设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` Roadmap（“脚本引擎（QuickJS）候选”）。承接：`DocLines`（按块读行，不整文件载入）、`begin_search/cancel`（协作取消）、`open/save` 文件对话框、`export_*` 的流式写盘模式。

**Goal:** 用户可在应用内管理并运行 `.js` 脚本，对**当前活动文档**逐行流式变换（保留/改写/丢弃），结果写入新文件；脚本有内存/时间/取消/输出上限，无文件系统/网络/require 能力。

**Architecture:** app 新增 `script.rs`：`rquickjs`（QuickJS，0.14）Runtime/Context 执行一次脚本源码，取全局 `onLine` 函数；逐行 `DocLines` → JS 调用 → `string|null` → `BufWriter`。命令 `script_list/read/write/delete/run`；运行复用 `begin_search` 取消登记。前端新增 `script-ui.ts` 模态（列表/编辑器/控制台），命令面板与工具栏“脚本…”。

**Tech Stack:** `rquickjs = "0.14"`（MIT；经 cc 编译 QuickJS C 源码，无运行时依赖）；workspace `rust-version` 1.79 → **1.87**。门：四门不变。

## 1. 范围与非目标
- 做：脚本目录管理（list/read/write/delete）；逐行变换（`return string` 保留/改写，`null`/`undefined` 丢弃，其它类型报错）；`console.*` 捕获；运行/取消/进度/统计；UTF-8 + LF 输出；原子写（tmp→rename）。
- 不做：`require`/ESM import/文件系统/网络/计时器/DOM；DBC 解码 API；`log.all()` 全量读取；多脚本管道；内置语法高亮编辑器（用 textarea，外部编辑器仍可直接改文件）；脚本签名/权限模型。

## 2. 脚本契约
```js
// 每个文档行调用一次（解码后的文本，不含行尾）。
// 返回 string 保留/替换该行；返回 null/undefined 丢弃。
function onLine(line, ctx) {
  // ctx = { lineNumber: number /*1 起*/, index: number /*0 起*/, path: string }
  return line;
}
```
- 源码 eval 一次；随后取全局 `onLine`；缺失/非函数 → 错误“脚本必须定义 onLine(line, ctx)”。
- 非 string/null/undefined 返回值 → 带行号的错误并停止。
- `console.log/info/warn/error` 捕获为控制台消息：参数以空格连接，对象先 `JSON.stringify`（失败则 `String`）；每条截断 4 KiB；**上限 200 条**，超限丢弃并追加一条“控制台输出已截断”。
- 无其它宿主全局（无 `require`、无 `process`、无文件/网络）。

## 3. app `script.rs`（Tauri 侧，非 core）
- 脚本目录：`$APP_CONFIG/LumeLog/scripts`。名字校验：`^[A-Za-z0-9_-]{1,64}\.js$`，禁止分隔符/`..`；命令一律用 `basenameOf` 后的名字定位文件。
- 常量：`SCRIPT_MEMORY_LIMIT = 64 MiB`、`SCRIPT_MAX_STACK = 1 MiB`、`SCRIPT_MAX_SECONDS = 300`、`SCRIPT_MAX_CONSOLE = 200`、`SCRIPT_MSG_CHARS = 4096`、`SCRIPT_MAX_OUTPUT = 4 GiB`、`PROGRESS_EVERY = 100_000` 行。
- `pub struct ScriptConsole { level: "log"|"warn"|"error", text: String }`；`ScriptProgress { scanned, written }`；`ScriptStats { scanned, written, dropped, cancelled, truncatedConsole: bool, error: Option<String> }`（camelCase DTO）。
- 运行器 `run_script(doc_arc, code, target_path, on_console, on_progress, cancel) -> ScriptStats`：
  1. `Runtime::new` + `set_memory_limit(64MiB)` + `set_max_stack_size(1MiB)`；`set_interrupt_handler` 检查 `cancel()` 或 `start.elapsed() > 300s`（返回 true 中断 JS）。
  2. `Context::full`；注入 `console`（四个函数把消息推入有界缓冲，达到上限后丢弃并置 `truncatedConsole`）；eval 源码；取 `onLine`。
  3. `DocLines::new(doc_arc)` 逐行：构造 `ctx` 对象（每次调用新建或复用并更新字段）；调用 `onLine`；`FromJs` 解析返回；`string` 写 `line + "\n"`（超 4 GiB → 错误），`null/undefined` 计 `dropped`。
  4. 每 `PROGRESS_EVERY` 行发 `on_progress`；`cancel` 在行间也检查；JS 异常/中断→ `error`（**中断且 `cancel=true` → `cancelled`；否则若已超 300s → `error="脚本超时（300s）"`**），停止并保留已写前缀。
  5. `BufWriter` 完成后 `flush`；运行器先写 `<target>.tmp-<pid>`，成功后 `rename` 覆盖目标，任何错误删除 tmp（目标不出现半文件）。
- 测试（`script::tests`）：名字校验（合法/分隔符/`..`/非 js）；控制台格式化与 200 条截断；`run_script` 用临时 doc + 临时输出：保留/改写/丢弃三态、行号 ctx、JS 抛错→error 且输出为前缀、缺 `onLine` 报错、UTF-8/LF 字节断言。

## 4. 命令与前端
- 命令：
  - `script_list() -> Vec<ScriptEntry{name,sizeBytes,modifiedMs}>`（目录缺失→空；坏文件跳过）
  - `script_read(name) -> String`；`script_write(name, text)`（建目录、tmp+rename）；`script_delete(name)`（缺失视为成功）
  - `script_run(doc_id, code, target_path, on_console: Channel<ScriptConsole>, on_progress: Channel<ScriptProgress>, on_done: Channel<ScriptStats>) -> reqId`：prologue 校验 doc 存在与名字/目标可写目录；`spawn_blocking` 运行；结束 `end_search`。
  - `script_cancel(req_id)`（复用 `state.cancel`）。
- `ipc.ts`：`ScriptEntry/ScriptConsole/ScriptProgress/ScriptStats` + `scriptList/Read/Write/Delete/Run/Cancel`。
- `script-ui.ts`：模态（复用 keymap 模态的遮罩/`inert` 模式）：
  - 左列：脚本列表（名字/大小/修改时间）+ “新建/删除”；右：`textarea`（monospace，Tab 插入两空格）+ “保存”；底部：控制台 `<pre>`（级别着色、自动滚底）+ 状态行 + “运行/取消/关闭”。
  - 新建：输入名字（模态内输入行，校验规则与服务端一致），初始内容为模板 `function onLine(line, ctx) { return line; }`；删除：确认；保存：`scriptWrite`；未保存切换/关闭提示（`ask` 确认）。
  - 运行：需活动文档；`save()` 选目标文件（默认 `<docBase>.out.txt`）；`scriptRun`（`onConsole` 追加行、`onProgress` 更新状态、`onDone` 汇总 toast/控制台）；运行中禁用编辑/运行，取消按钮调 `scriptCancel`。
- `main.ts`：面板命令 `{ id:"script", title:"脚本…", run: openScriptManager }`（不进 keymap 可绑动作集）+ 顶部工具栏“脚本”按钮；`openScriptManager` 需保证 `docId` 可选（无文档也能管理与编辑脚本，只是“运行”禁用并提示）。**模态打开时 `scriptModalOpen=true`，全局快捷键分发短路（与 keymap 模态同样处理）；两个模态互斥。**
- `styles.css`：`.script-modal/box/list/editor/console/...` 复用 token；z-index 与 keymap 模态一致（85），两模态互斥。

## 5. 错误 / 边界
- 脚本名非法/读不到：`invalid_argument`/`io`；JS 语法错误、运行错、缺 `onLine`、返回类型错：`script_error`（含行号，`on_done.error` + 控制台 error 行）。
- 内存/栈/时间超限：QuickJS 中断 → 错误“脚本超出限制/已取消”；输出 >4 GiB → 错误；目标写失败 → 错误 + 删除 tmp。
- 取消：行间 + JS 中断双检查；`cancelled=true`，保留前缀；`on_done` 必发。
- 目标文件与源文档同路径：拒绝（防止覆盖正在读的文档）。
- 输出编码固定 UTF-8/LF（源编码不影响；文档注明）。
- 活动文档可为编辑/只读模式；`DocLines` 一致。
- 大文件：逐块读（`MAX_CHUNK_LINES`），内存恒定；控制台与进度回调有界。

## 6. 验收
- **Rust**：`script::tests`（见 §3）+ `cargo test --workspace`；`cargo clippy --workspace --all-targets -- -D warnings`。
- **前端**：`script-ui.test.ts`（jsdom：列表渲染、新建/保存回调、运行参数与 onConsole 追加）；`ipc.test` typeof；`npm test` + build。
- **GUI 手工**：脚本管理器新建/保存/删除；运行一个“丢弃空行 + 改前缀 + 统计行号”的脚本→输出文件正确；语法/运行错误显示且不崩；运行中取消保留前缀；内存炸弹/死循环在 300s 内被中断；无文档时运行禁用。

## 7. 全局约束
简体中文 UI；DTO camelCase；逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；push；`--no-ff` 合 main。分支基 `main`（当前 `35078d3`）→ `feat/roadmap-script`。MSRV 随 rquickjs 0.14 升至 1.87（CHANGELOG 注明）。

## 8. 分期（计划预告）
① MSRV 1.87 + `rquickjs` 依赖 + `script.rs` 名称/控制台工具 + 测 → ② `run_script` 运行器 + 临时文档测试 → ③ 命令/DTO/注册 → ④ 前端 ipc → ⑤ `script-ui.ts` 模态 + jsdom 测 → ⑥ `main.ts` 接线 + 样式 → ⑦ 四门+终审+合 main。

## 9. 关键取舍（已定案）
- **逐行流式变换**（你选）：恒定内存、可取消、GB 级可用；不提供 `log.all()`。
- **脚本管理器 + 控制台**（你选）：列表/编辑/保存/删除 + 运行控制台；不内置高级编辑器。
- **rquickjs 0.14**（你选）：MSRV 1.79→1.87；QuickJS 经 cc 编译，无运行时依赖。
- **无宿主 fs/net/require**；`console` 有界；内存 64MiB、栈 1MiB、单次 300s、输出 4GiB 上限。
- 输出固定 UTF-8 + LF；目标与源同路径拒绝；tmp→rename 原子写。
