# 编辑器工具三件套：宏录制 / 概览条（文档地图）/ 函数列表 设计（Plan 29）

spec §5 缓议项开工。全部前端改动，Rust 零改动。共享原则：逻辑拆纯函数进
`editortools.ts`（可测），CodeMirror 侧只做粘合。

## 1. 宏录制

- 记录**用户输入事务**（`tr.isUserEvent("input.type" | "input.replace" | "delete.backward" | "delete.forward")`
  且 docChanged），每个事务的 `iterChangedRanges` 折叠为操作序列：
  `{ kind: "delete", count }` / `{ kind: "insert", text }`（纯函数 `extractMacroOps`）；
- 播放：从**当前光标**顺序重放（insert 在光标插入并前移；delete 向后删 min(count, 剩余)），
  每个操作一次 dispatch（可整段撤销）；上限 10,000 操作防失控录制；
- 已知限制（与 N++ 键盘宏同级的脆弱性，明确记录）：重放是相对光标的字节级重放，
  不记录选区/多光标/鼠标点击；内容不同则结果不同；
- 交互：命令面板「开始/停止录制宏」「播放宏」（状态用 toast 提示，N++ 亦无持久指示灯）；
  面板同款快捷键不占 keymap 动作位（palette-only，避免与 F2 系冲突）。

## 2. 概览条（文档地图）

- 编辑器右缘 36px 竖条 canvas（overlay，`.cm-editor` 内 absolute）；仅编辑模式；
- 绘制：行数 ≤ 高度像素时一行一像素行；否则按 `overviewBuckets(totalLines, px)` 分桶，
  桶内非空行计数映射灰度（日志类文档的空行纹理即"文档形状"）；
- 视口窗口：按 `view.visibleRanges` 画半透明 accent 矩形；`docChanged`/`viewportChanged`
  时 rAF 合并重绘；行数 > 50 万只画视口框（性能护栏）；
- 交互：点击/拖拽跳行（`lineAtY` 纯函数换算）；
- 纯函数：`overviewBuckets / lineAtY`（单测）；开关默认开，命令面板切换（不持久化）。

## 3. 函数列表

- `SYMBOL_PATTERNS`：按内置模式 id 的正则表（js/ts：function/箭头常量/class；python：def/class；
  rust：fn/struct/enum/trait；java/c/cpp：类与函数签名；markdown：标题；sql：CREATE TABLE/VIEW/FUNCTION）；
  未收录模式 → 空表（面板提示「当前语法无符号规则」）；
- `extractSymbols(lines, patterns)` 纯函数：`{ name, line }`（1-based），同行多次命中取首个；
- 面板：`openCommandList(title, commands)` —— 把 openPalette 的 overlay 构建泛化复用，
  函数列表以符号为 Command（hint = 行号，run = revealLine + focus），输入框即符号过滤；
- 只读文档/无符号 → toast 提示。

## 4. 验收

- `editortools.test.ts`：宏提取（混合插删）、概览分桶边界、符号提取各语言样本；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；Rust 零改动；
- 人工：录制→编辑→播放、大文件概览条拖拽、函数列表跳转。
