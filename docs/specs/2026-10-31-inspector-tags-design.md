# 检查器：文档属性分区 + 文档标签（Plan 33）

对齐参考项目 NoteLume 的检查器（截图：文档属性 / 标签两块）。LumeLog 右侧
信息栏已有属性行（模式/编码/行尾/行数/已索引/无法映射/路径），本 plan
把它分成「文档属性」「标签」两个小节，属性补大小与修改时间，标签为
按路径持久化的文档标注。全部前端改动，Rust 零改动。

## 1. 文档属性

- 现有 rows 重组为「文档属性」小节；新增两行：大小（`humanSize`）、
  修改时间（`formatSavedAt(info.mtimeMillis)`）——两个字段在 `DocInfo`
  上现成，不进 `DocState`（大文件重载会整体刷新，从活动 tab 取）；
- 无法映射字符 > 0 仍黄色警示；无打开文档仍显示「无打开文档」；
- 大小/修改时间取打开时刻（存 `DocState`，外部重载时刷新）；tail 跟随
  增长后到下次重载前不回刷，属已知限制。

## 2. 文档标签

- 存储：`localStorage["lumelog.doc-tags.v1"]`，`Record<path, string[]>`
  （键用路径——标签属于文件而非本次打开的会话）；上限每文档 20 个、
  单标签 32 字符（trim 后）；坏数据逐文档丢弃；
- 纯函数进 `doctags.ts`（parse/serialize/normalize/add/remove），
  存储读写与 DOM 在 `main.ts`；
- UI：小节内 chips + 末尾输入框，回车添加（非空、未超限、未重复），
  chip 上 × 删除，操作即落盘；无文档不渲染标签区。

## 3. 验收

- `doctags.test.ts`：normalize（trim/空/超长截断）、add（重复/满额/非法）、
  remove、parse（坏 JSON/坏条目丢弃/上限钳制）、序列化往返；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；
- 人工：标签增删刷新后仍在（`tauri dev`）。
