# 宏补遗：真实方向键/选区扩展录制 + 跳词移动与跳词删除（Plan 32）

Plan 30/31 留下的三块限制（选区、多光标、跳词）的收口。全部前端改动，
Rust 零改动。

## 0. 先修一个录制 bug

核对 CodeMirror 源码发现：方向键移动命令的 userEvent 是 `select`，
Plan 29 spec 写的 `input.move` 只在我们测试的合成事务里出现——
真实键盘移动从未进过宏。本 plan 把捕获路由改为
`isUserEvent("select")`（保留 `input.move` 兼容测试合成事务）。

## 1. 选区扩展录制

- `select` 事件且不改文档时：锚点不变、主光标单步/边界/相邻行移动 →
  记 `{ kind: "move", by, select: true }`（复用 `classifyMove`）；
  锚点变化（点击）、大幅跳转（翻页、Ctrl+Home、shift 点选）不录；
- 重放：select 版命令（selectCharLeft 等），行首/行尾 dispatch
  `{ anchor: 原锚点, head: 边界 }`；
- 重放 delete 时若选区非空则删整个选区（对应"选中一段再输入"的
  替换模式，录出来是 delete+insert 对）。

## 2. 跳词移动与跳词删除

- `cursorGroupLeft`（Ctrl+←）与 Backspace 的 Ctrl 版（`deleteGroupBackward`）
  产生的事件与普通移动/删除完全同名，无法从事务区分 → 键位拦截：
  buildState 的 keymap 在 defaultKeymap 之前加同键位绑定
  （Mod-ArrowLeft/Right、Alt-ArrowLeft/Right、Mod-Backspace、Mod-Delete，
  含 mac 映射与 shift 处理，与默认行为逐一对应），录制中先记
  `{ kind: "move", by: "word-left"|"word-right" }` 或
  `{ kind: "del-word", dir }`，再调用默认同款命令；
  非录制时同样先过我们的绑定再跑默认命令，行为不变；
- 拦截期间置抑制标志，防止同一次 dispatch 再被 select/delete 捕获重复记录；
- 重放：word-left/right → cursorGroupLeft/Right；del-word →
  deleteGroupBackward/Forward。

## 3. 多光标：明确不做

动作流按主光标建模，忠实重放需对每个光标独立重放再合并结果，
与变更映射、撤销强耦合；N++ 的多光标宏同样受限。录制期间事务
涉及多选区直接跳过（不录垃圾数据）。

## 4. 限制

- 跳词的 select 变体（shift+Ctrl+←）不录（行为保留，不进宏）；
- Ctrl+Home/End、翻页、点击不录；
- 跳词重放统一走 cursorGroup 系，Alt（语法感知）录制与重放有细微差别。

## 5. 验收

- `editor.test.ts`：shift 选区 ×2 → 替换输入的录制与重放全文一致；
  键位拦截录 del-word 并重放（jsdom keydown）；
- 既有 29 个测试不回归；`tsc --noEmit` + `npm test` + `npm run build` 绿；
- 人工：实机方向键/选区/跳词录制手感（`tauri dev`）。
