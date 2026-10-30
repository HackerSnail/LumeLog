# 插件接口 增量2：QuickJS 数据解析插件 设计（Plan 18）

接 Plan 17（声明式高亮）。对应 `docs/features.md` 待做第 1 条的「冷门日志与报文格式」：
用户以 JS 插件描述一个**逐行解析器**，把任意冷门日志/报文格式解析成结构化表格，不改主程序。
Wasm host 仍为增量3。

## 1. 插件形态

`$APP_CONFIG/LumeLog/plugins/*.js`（与语法插件 JSON 同目录，按扩展名区分种类）：

```js
var plugin = {
  id: "myfmt",                 // 1–64 字符 [A-Za-z0-9_-]，规则与语法插件一致
  name: "My 格式",              // 显示名 1–64
  extensions: ["mylog"],       // 1–16 个，AUTO 预选用
  columns: [                   // 1–32 列，顺序即表格列序
    { key: "time",  label: "时间" },
    { key: "level", label: "级别" },
  ],
  parseLine(line, ctx) {       // ctx = { lineNumber, index, path }（与脚本引擎一致）
    return { time: "...", level: "..." };   // 对象：逐列取值；null/undefined：跳过该行
  },
};
```

- 顶层 `var plugin`（`const/let` 走 script.rs 同款回退 eval 读取）。
- id 规则与语法插件相同（1–64、`[A-Za-z0-9_-]`）；两类插件注册表不同，同名互不冲突
  （各自列表内按文件名序 first wins）。
- 列值接受 `string/number/boolean`（`number/boolean` 转字符串）；`null/undefined` → 空串；
  其它类型 → 空串并在 console 记一条 warning（每插件限频）。
- 沙箱与脚本引擎同参：64MiB 内存 / 1MiB 栈 / 无 fs/net/require、console 注入（ConsoleBuf 上限同）。
  元数据加载阶段超时 5s；`parser_run` 总超时 300s + 可取消。

## 2. Rust（`plugins.rs` 扩展 + 新执行函数）

- `MAX_PLUGIN_JS_BYTES = 1MB`（源码，与脚本同限）。
- `list_parser_plugins(dir) -> (Vec<ParserPluginInfo>, Vec<String>)`：对每个 `*.js`
  （文件名序、单文件失败不阻塞、id 重复后到者跳过——与语法插件同规矩）在 rquickjs 里执行一次，
  读取 `plugin` 元数据并校验；`parseLine` 必须是 function。
- `ParserPluginInfo { id, name, extensions, columns: Vec<ParserColumnJson{key,label}> }`。
- `run_parser_plugin(doc, plugin_path, on_batch, on_console, cancel)`：
  `DocLines` 逐行 → `parseLine(line, ctx)` → `ParserRowJson{ line, cells }`
  （cells 按 columns 顺序、全部字符串化）→ 512 行/批 emit（`can_parse` 同策略）；
  结束发 `ParserStats{ rows, lines, skipped, cancelled, error }`。
  行数上限沿用文档本身，不做额外物化（行数据 O(rows) 驻留给前端 store，同 CanPanel 前例）。
- 命令（照 `can_parse` 异步双通道模式）：
  - `parser_plugins_list(app) -> { parsers, errors }`；
  - `parser_run(state, app, doc_id, plugin_id, on_batch, on_done) -> req_id`：
    后端**按 id 重新扫描目录**定位文件（前端不传路径，防注入）；
    `spawn_blocking` + `catch_unwind` + `begin_search/end_search` 取消注册；
  - `parser_cancel(request_id)`。
- 新错误码：`plugin_not_found`（id 不存在）、`plugin_bad_manifest`（元数据缺列/缺 parseLine）。

## 3. 前端

- `ipc.ts`：`ParserPluginInfo/ParserRowJson/ParserStats/ParserPluginsReply` + 三个包装 +
  错误码表补 2 条（`IpcErrorCode` 联合 + `IPC_ERROR_ZH`）。
- 新 `parserpanel.ts`：照 `canpanel.ts` 骨架裁剪——行存储（rows 累加器）、`@tanstack/table-core`
  列模型**按插件 columns 动态生成**、自写虚拟滚动（镜像 `listview` 的 `ROW_HEIGHT` +
  可视窗 ± buffer + rAF 节流）、行点击回调 `onRowActivate(lineNumber)`。
  无过滤/无导出/无列显隐（留后续增量）；状态行显示 `已解析 N 行 · 跳过 M 行`。
- `main.ts`：工具栏「解析」按钮 + `buildParserDock()`（与 CAN/MF4 dock 互斥，同 `toggleCanDock`
  模式）；dock 内：插件下拉（`parser_plugins_list`，按当前文档扩展名预选）+「解析/停止」+
  面板宿主；token 丢弃旧响应；行点击 `revealSourceLine`；关闭时 `parser_cancel` + destroy。

## 4. 非目标

- 不过滤、不导出 CSV/JSONL、不存会话（CanPanel/MF4 的能力按需后续搬）；
- 不支持多行记录/块解析（`parseLine` 逐行模型；多行格式留给增量3 Wasm 或后续扩展 ctx）；
- 解析结果不落盘、不参与搜索（表格内存态，重解析即重建）；
- 不做插件间依赖/导入。

## 5. 验收

- Rust：元数据加载/校验（缺 plugin、缺 parseLine、列超限、坏 JS）、`run_parser_plugin`
  端到端（临时 doc + 插件：行→cells、null 跳过、number 转 string）、取消、未知 id；
  `cargo test --workspace` + `clippy -D warnings` 净。
- 前端：`parserpanel` jsdom 测试（列头、批量 push、行点击回传行号、destroy 停止渲染）；
  `npm test` + `npm run build` 绿。
- 人工：`npm run tauri dev` 放示例插件 → 解析 → 点行跳源行。
