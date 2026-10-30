# 插件接口 增量1：声明式语法高亮插件 设计（Plan 17）

对应 `docs/features.md` 待做第 1 条：「插件接口，支持用户扩展解析器：冷门编程语言的高亮/解析、
冷门日志与报文格式，都应能以插件形式补入，不改主程序」。

## 1. 总体路线（分三个增量，本 plan 只做增量1）

| 增量 | 内容 | 载体 |
|---|---|---|
| **增量1（本次）** | 冷门语言/日志的**高亮**：声明式 JSON 插件 | 前端 `StreamLanguage`，无代码执行 |
| 增量2 | 冷门**数据格式**解析（CAN 源、自定义导出）：JS 逻辑插件 | 复用 `rquickjs` 沙箱（脚本引擎同栈） |
| 增量3 | 重解析器/二进制格式：Wasm host | wit 接口（候选 Extism 或自研 host） |

理由：高亮是最自然的挂载点（`language.ts` 已是集中注册表），且声明式插件**不执行用户代码**，
不需要先解决沙箱在前后端之间的位置问题；数据解析插件必须跑在后端沙箱里，另立 plan。

## 2. 插件形态

目录 `$APP_CONFIG/LumeLog/plugins/*.json`（与 `scripts/` 平级），一个文件一个语法：

```json
{
  "id": "plc-log",
  "name": "PLC 日志",
  "extensions": ["plc", "plclog"],
  "lineComment": "#",
  "keywords": ["START", "STOP", "ALARM"],
  "rules": [
    { "regex": "^\\d{2}:\\d{2}:\\d{2}\\.\\d{3}", "tag": "className" },
    { "regex": "0x[0-9a-fA-F]+", "tag": "number" }
  ]
}
```

- `id`：1–64 字符，`[A-Za-z0-9_-]`，全局唯一；模式 id 为 `plugin:<id>`。
- `name`：状态栏/下拉显示名，1–64 字符。
- `extensions`：1–16 个，每个 1–16 字符 `[A-Za-z0-9_-]`（大小写不敏感）。
- `lineComment`：可选，1–8 字符、无空白；命中后整行 comment。
- `keywords`：可选，≤512 个，每个 1–64 字符、无空白，按词边界匹配。
- `rules`：可选，≤32 条，按序尝试；`regex` 非空 ≤512 字符；`tag` 限 7 个
  （`className / keyword / number / string / comment / propertyName / operator`，
  与 `streamHighlight` 同一组，`defaultHighlightStyle` 全部覆盖）。
- 未知字段拒绝（`deny_unknown_fields`，防拼错字段静默失效）。
- 上限：插件文件 64 个 / 单文件 256KB。

## 3. 职责划分

- **Rust（`app/src-tauri/src/plugins.rs`）**：读取目录、JSON 解析、schema 校验、
  id 去重（按文件名排序，后到者报错跳过）、单文件失败不阻塞其它（errors 逐条回报）。
  命令 `syntax_plugins_list -> { plugins, errors }`。
- **前端**：把 manifest 编译为 `StreamLanguage`（正则用 JS `RegExp` sticky 锚定当前流位置；
  Rust `regex` 与 JS 方言不同，编译校验放前端，失败的单条规则跳过并回报 warning）。
  `language.ts` 扩展注册表：插件模式进独立 map，`autoModeId = 内置扩展名 → 插件扩展名`
  （内置优先，插件不劫持），`resolveLanguage("plugin:<id>")` 直接命中已编译实例。

## 4. 交互

- 启动时加载一次；命令面板新增「重载语法插件」（palette-only，不占 keymap 动作位）。
- 状态栏「语法」下拉 = 内置 21 项 + 插件模式（显示 `name`，值 `plugin:<id>`），
  由新 API `Statusbar.setSyntaxModes()` 重建，保持当前选中。
- 重载只影响新开文档与手动切换，不重建已打开编辑器（避免打断编辑）。
- 加载失败汇总为一条 toast，明细进 `console.warn`。

## 5. 非目标（增量1）

- 不执行插件内代码（无 JS/Wasm 执行）；不支持块注释/嵌套规则/多行正则
  （`StreamLanguage` 是行内流模型，`^` 只在行首有意义）；
- 不持久化「每个文档手动选过的插件模式」（与内置模式现状一致，靠扩展名自动匹配）；
- 不做插件市场/签名/远程下载。

## 6. 验收

- Rust：合法/非法 manifest、上限、坏 JSON 跳过、id 重复、目录缺失 → 单测；
  `cargo test --workspace` + `clippy -D warnings` 净。
- 前端：注册后 `resolveLanguage("plugin:<id>")` 出 `LanguageSupport`、`autoModeId` 内置优先、
  坏正则跳过并回报、statusbar 重建下拉且回传 id；`npm test` + `npm run build` 绿。
- 人工：放一个示例 JSON → 状态栏出现新模式、对应扩展名文件自动高亮。
