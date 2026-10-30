# 语法高亮语言扩展 设计（Plan 15）

对应 `docs/features.md` 待做第 2 条：「语法高亮扩展到各类编程/配置语言（目前仅 cpp/json 等少数）」。

## 1. 背景与现状

`app/frontend/src/language.ts` 是唯一的语法模式分发点，当前只有 5 种：

| 模式 | 来源 | 扩展名 |
|---|---|---|
| `log` | 自建 `StreamLanguage` | log |
| `ini` | 自建 `StreamLanguage` | ini / cfg / conf |
| `json` | `@codemirror/lang-json` | json |
| `c` | `@codemirror/lang-cpp` | c / h / cpp / cc |
| plain | null | 其余 |

`AVAILABLE_MODES` 是状态栏下拉的唯一数据源；`main.ts applyMode` 把模式名当扩展名传给
`handle.setMode(ext)`，再由 `modeForExtension(ext)` 解析。也就是「模式 id 与扩展名共用一套字符串」，
新增语言必然要动这个隐含耦合。

## 2. 目标 / 非目标

目标：

1. 常见编程与配置语言开箱高亮：JS/TS、Python、HTML、CSS、XML、Markdown、SQL、Rust、Java、
   YAML、TOML、Shell、Diff、PowerShell；
2. 新增一门语言的改动面收敛为「注册表一行 + （必要时）一个依赖」；
3. 不因加语言把首屏包体撑大（轻量是立项前提）。

非目标：

- 只读虚拟列表（>200MB）的高亮——只读路径没有 CodeMirror，本次不动；
- LSP / 补全 / 格式化 / 折叠语义增强，`lang-*` 自带折叠与缩进即够；
- 用户自定义语言（语法高亮的自定义归插件接口那条待做）；
- 后端（Rust）任何改动。

## 3. 设计决策

### 3.1 模式 id 与扩展名解耦

引入注册表：`mode id → { 扩展名集合, 显示名, 加载器 }`。对外只暴露三个纯语义函数：

- `modeIdForExt(ext): ModeChoice | null` —— 同步、纯函数，扩展名 → 模式 id（未知 → null = 纯文本）。
  状态栏 `AUTO` 与 `applyMode` 都走它，可单测且不需要加载任何语言包。
- `resolveLanguage(mode): Promise<LanguageSupport | null>` —— 按 id 动态加载并缓存。
- `AVAILABLE_MODES` / `MODE_LABELS` —— 下拉的取值与显示名（显示名不再靠 `toUpperCase()` 硬凑）。

`applyMode` 由「传扩展名」改为「传模式 id」，`editor.ts setMode` 同步改签名，消除原隐含耦合。

### 3.2 语言包按需动态加载

9 个 lezer 语法包静态全量引入会把首屏 bundle 从约 620KB 推到 1MB 量级，与「轻量快速」定位冲突。
因此语言包一律 `await import()`，加载到 `Map<ModeChoice, LanguageSupport>` 缓存；`log` / `ini`
是本地 `StreamLanguage`，同步返回不产生 chunk。

代价是 `setMode` 变异步，需防竞态：同一个 editor 保存自增 token，旧请求回包落后于新请求则丢弃，
避免快速连切语法时高亮停在旧语言上。

### 3.3 配色

`editor.ts` 已常驻 `syntaxHighlighting(defaultHighlightStyle, { fallback: true })`：lezer 语言
（json/c/js/py/…）与新接入的 legacy `StreamLanguage` 都只发 tag、不带样式，因此统一由它着色，
无需为每种语言补 `HighlightStyle`。`log` / `ini` 原有的 `streamHighlight` 是非 fallback，
随语言一起挂载，行为不变。

### 3.4 语言与扩展名清单

官方 lezer 包（新增依赖）：

| 模式 id | 显示名 | 包 | 扩展名 |
|---|---|---|---|
| javascript | JavaScript | `@codemirror/lang-javascript` | js mjs cjs jsx |
| typescript | TypeScript | 同上（`{typescript:true}`） | ts tsx mts cts |
| python | Python | `@codemirror/lang-python` | py pyw pyi |
| html | HTML | `@codemirror/lang-html` | html htm |
| css | CSS | `@codemirror/lang-css` | css |
| xml | XML | `@codemirror/lang-xml` | xml xsd xsl svg plist resx |
| markdown | Markdown | `@codemirror/lang-markdown` | md markdown |
| sql | SQL | `@codemirror/lang-sql` | sql |
| rust | Rust | `@codemirror/lang-rust` | rs |
| java | Java | `@codemirror/lang-java` | java |

`@codemirror/legacy-modes`（单包多模式，`StreamLanguage` 包裹，体积远小于 lezer）：

| 模式 id | 显示名 | 子路径 | 扩展名 |
|---|---|---|---|
| yaml | YAML | `mode/yaml` | yaml yml |
| toml | TOML | `mode/toml` | toml |
| shell | Shell | `mode/shell` | sh bash zsh |
| diff | Diff | `mode/diff` | diff patch |
| powershell | PowerShell | `mode/powershell` | ps1 psm1 psd1 |

`c` 的扩展名补 `hpp / cxx / hh`。`AUTO`、`Plain Text`、`Log`、`INI`、`JSON` 五项的 id 与行为保持不变。

## 4. 验收

- 新增 `language.test.ts`（jsdom）：扩展名映射、每个 `AVAILABLE_MODES` 项都能解析出
  `LanguageSupport`（防注册表漂移）、缓存命中同一实例、`plain`/未知 → null。
- `statusbar.test.ts` 增 1 例：下拉显示名与回调取值为模式 id。
- `npm test` 全绿；`npm run build`（含 `tsc --noEmit`）通过；比较构建前后 `dist/index-*.js` 体积，
  首屏主 chunk 不得因本次改动增长（语言落在独立 chunk）。
- Rust 侧无改动，`cargo test --workspace` / `clippy -D warnings` 不回归。
