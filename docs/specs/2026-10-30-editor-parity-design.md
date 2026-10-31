# 编辑器对齐 Notepad++：行操作/换行切换/字号缩放/选区高亮 设计（Plan 27）

盘点现状：查找/替换面板（`@codemirror/search`，Ctrl+F/H、正则/大小写/整词/全部替换）、
转到行（Ctrl+G `openJumpPrompt`）、行复制/移动/删除（defaultKeymap 自带）、
语法高亮 19 种、多标签、自动换行（**常开**）已就位。本 plan 补齐其余高频差异，全部前端改动。

## 1. 行操作（命令面板条目，作用于选区覆盖的行；无选区 = 整篇）

`transformLines(view, fn)`：取行区间文本 → `fn` → 一次 `dispatch`（单步撤销）。

| 命令 | fn |
|---|---|
| 行排序（升/降） | `localeCompare` |
| 行去重 | 保首个出现的行 |
| 行反转 | 逆序 |
| 转大写 / 转小写 | 全选区行文本 |
| Trim 行首尾空格 | `trim()` |
| 去空行 | 过滤 `trim() !== ""` |
| 合并为一行 | 以空格连接（N++ Join Lines） |

空结果保护：除"去空行/合并"外，输出行数不变；排序等在单行/空文档上是 no-op（仍走 dispatch 无妨）。

## 2. 换行显示切换（现常开，改为可切换 + 持久化）

- `EditorView.lineWrapping` 移入 Compartment；`setWrap(on)` 切换；
- 开关持久化 `localStorage "lumelog.ui.wrap"`（默认开，与现状一致），`openEditable` 经 opts 接收初值；
- 命令面板「切换自动换行」，状态栏不加控件（面板 + 快捷键足够，不挤占状态栏）。

## 3. 字号缩放（Ctrl+滚轮 + 命令面板）

- 字号 Compartment：`EditorView.theme` 动态设置 `fontSize`，基准 14px，步进 ±1，钳制 9–28；
- 编辑器容器 Ctrl+滚轮缩放（`EditorView.domEventHandlers`，`preventDefault`）；
- 命令面板「放大字体/缩小字体/重置字体」；不做持久化（会话级，与 N++ 行为一致）。

## 4. 选区匹配高亮

`@codemirror/search` 的 `highlightSelectionMatches`（选中词的所有出现高亮）加入基础扩展。

## 5. 不做（后续按需）

书签栏（gutter + StateEffect，涉及状态持久化）、列选择模式、宏录制、文档地图、
函数列表、查找结果面板对接只读虚拟列表（只读路径的搜索已有行过滤体系）。

## 6. 验收

- `transformLines` 与 `clampFontSize` 纯函数单测；换行切换 facet 断言；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；Rust 零改动；
- 人工：选区行排序/去重、Ctrl+滚轮缩放、换行切换后包行变化。
