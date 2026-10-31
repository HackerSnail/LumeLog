# 书签跨会话持久化（Plan 38）

Plan 28 遗留（spec §3）：书签随文档关闭丢失。跨会话持久化按**文件路径**存
**行号**（N++ 同款语义）——字符偏移隔了一次磁盘编辑就失效，行号才稳定；
文件变短后越界行号装载时丢弃。

## 1. 存储（前端）

- `localStorage["lumelog.bookmarks.v1"]`：`Record<path, number[]>`
  （1-based 行号升序去重，每文档 ≤200 条）；坏数据逐文档丢弃；
  纯函数进 `bookmarks-store.ts`（parse/serialize），存储 IO 在 `main.ts`；
- 写入时机：编辑器书签集合每次变化（F2 切换 / 清除）整表替换该路径条目，
  空表删键——不做关闭时一次性落盘（崩溃也不丢）。

## 2. 编辑器

- `handle.bookmarkLines()`：锚点 → 1-based 行号（升序）；
- `handle.loadBookmarkLines(lines)`：越界行号丢弃，行号 → 行首锚点
  经既有 `setBookmarks` effect 批量装载；
- `OpenEditableOpts.onBookmarksChanged(lines)`：切换/清除后回调
  （跳转不改集合不回调）；`main.ts` 挂到 mountDoc：打开后装载
  该路径已存行号，变化时整表落盘。只读视图无书签（edit-only），不涉及。

## 3. 验收

- `bookmarks-store.test.ts`：parse（坏 JSON/去重排序/上限/负数丢弃）、往返；
- `editor.test.ts`：loadBookmarkLines 越界钳制 + 锚点就位；
  toggle/clear 触发 onBookmarksChanged 且行号正确；跳转不触发；
- tsc/npm test/build 绿；人工：打书签→关文档→重开仍在（`tauri dev`）。
