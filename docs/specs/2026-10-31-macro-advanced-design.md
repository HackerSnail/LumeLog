# 宏高级形态：编辑动作抽象 + 跨会话持久化（Plan 30）

Plan 29 宏录制的续作。全部前端改动，Rust 零改动。纯函数继续进
`editortools.ts`，CodeMirror 粘合在 `editor.ts`，存储与命令面板在 `main.ts`。

## 1. 编辑动作抽象

Plan 29 的宏只有字节级 `{insert|delete}`，重放完全依赖"光标恰好停在同样
的相对位置"。本 plan 把操作模型升级为四类动作：

- `{ kind: "insert", text }` —— 不变；
- `{ kind: "delete", count, dir: "forward"|"backward" }` —— 补方向：
  Backspace（`delete.backward`）重放时向光标前删，Delete 向后删，
  不再把 Backspace 错放成向后删；
- `{ kind: "move", by: "left"|"right"|"up"|"down"|"line-start"|"line-end" }`
  —— 光标移动进入宏。录制捕获 `input.move` 且不改文档的事务，
  纯函数 `classifyMove(before, after, afterLineLen)` 按前后行列坐标归类：
  同行 ±1 列 → left/right；行号 ±1 → up/down；同行归到行首/行尾
  （列差 >1 才算 Home/End，±1 的归 left/right）；其余（跳词、翻页、
  跨行折行边界）不录，属已知限制；
- `{ kind: "line", op: LineOp }` —— 行批处理命令（Plan 27 的九种）进入宏：
  `handle.lineOp` 在录制中把命名的行操作记进去，重放按当前选区（空选区
  整篇）执行，语义与命令面板一致。

录制上限沿用 1 万操作。重放依旧每个操作一次 dispatch。

已知限制（记录在案）：不录选区/多光标/鼠标点击；跳词移动不进宏；
行操作重放取重放时的选区，与录制时不必相同。

## 2. 跨会话持久化

- 保存格式：`{ v: 1, macros: [{ name, ops }] }` JSON，
  存 `localStorage["lumelog.macros.v1"]`（UI 偏好既有模式；
  脚本走 `$APP_CONFIG` 是因为内容可能很大，宏没有这个量级）；
- 纯函数 `serializeMacros` / `parseMacros`：解析逐条校验形状，
  坏宏丢弃不拖垮整个库；上限 50 个宏（`MACRO_STORE_MAX`）、
  名字 100 字符、每宏 1 万操作；损坏 JSON 返回空表；
- 命令面板新增 3 条：
  「宏：保存为…」——泛化列表面板加 `onFreeText`（列表为空时回车提交
  输入框文本），同名覆盖；
  「宏：运行已保存宏」——列出已存宏（hint 操作数），运行时把 ops
  交给 `playMacro(1, ops)`；
  「宏：删除已保存宏」——同列表删除并立即落盘。
- 停止录制后操作序列留在会话内（handle 只读 `macroOps()`），
  保存即从这份序列取名落盘。

## 3. 验收

- `editortools.test.ts`：classifyMove 各分支、extractMacroOps 方向、
  serialize/parse 往返 + 坏数据 + 上限；
- `editor.test.ts`：录制（插入 + 移动 + 前删 + 行操作）→ 重放全文一致；
  `playMacro` 显式传入 ops；`macroOps()` 返回已录制序列；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；Rust 零改动；
- 人工：保存→重启→运行已保存宏（跨会话链路只有 `tauri dev` 能验）。
