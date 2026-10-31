# 书签栏 设计（Plan 28，spec §5 开工第一项）

spec §5 五项盘点：**列选择模式已随 `rectangularSelection()` 交付**（Alt+拖拽块选，无需开发）；
书签栏本 plan 实现；宏录制、文档地图、函数列表继续缓议（工作量/价值比差）。

## 1. 数据模型

- 编辑器实例内 `StateField<readonly number[]>`：**字符偏移锚点**（排序、去重），
  随 `tr.changes.mapPos` 自动跟随编辑（增删行不丢书签）；
- 行装饰：书签行加 `cm-bookmarked-line`（subtle 背景条）；
- gutter：独立 `cm-bookmark-gutter`，`lineMarker` 按锚点命中行返回圆点 marker（accent 色）。

## 2. 操作

| 操作 | 行为 |
|---|---|
| 切换书签 | 光标所在行加/删（Ctrl+F2，N++ 同款） |
| 下一条 / 上一条 | 从光标行向后/向前找最近书签，环绕；跳转并居中（F2 / Shift+F2） |
| 清除全部 | 命令面板条目 |

- 纯函数 `bookmarkToggle/next/prev`（有序锚点数组 + 光标偏移 → 新数组/目标偏移）放
  `editorops.ts`，单测覆盖增删、环绕、空书签、多光标取主光标；
- handle 增 `toggleBookmark/nextBookmark/prevBookmark/clearBookmarks`；
- 命令面板加 4 条；CodeMirror keymap 加 F2 系（与 app 级 keymap 无冲突，F2 未占用）。

## 3. 限制

书签为**会话内**状态：随文档关闭丢失（N++ 经 session 文件跨会话保存——后续可挂到
恢复/会话文件，本 plan 不做）。多光标只取主光标行。

## 4. 验收

- `bookmarkToggle/next/prev` 纯函数单测（增删、环绕、单元素、空表）；
- editor 集成测试：toggle → gutter/装饰状态可经 handle 断言、F2 环绕跳转选中行正确；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；Rust 零改动；人工观感 `tauri dev`。
