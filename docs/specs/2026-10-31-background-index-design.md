# 后台流式索引：threaded doc_open + 8 MiB 进度 tick（Plan 37）

M1 遗留欠账（test-checklist §4）：GUI 侧占位行/refresh/Channel 全部就绪
（listview.ts 注释即写着 "streams ahead of it in Task 11"），只差 shell 侧
把同步建索引改成后台线程。目标：只读大文档打开即首屏（估算行数 + 占位行），
索引边建边刷。

## 1. core：可增量索引的 DocFile

- `DocFile::open` 重构为 `open_deferred`（探测编码/行尾/签名，索引空：
  offsets=[0]、file_size=0）+ 循环 `index_step`——CLI/reload 语义不变；
- `index_step(chunk)`：按 READ_CHUNK 顺序读一块，非 UTF-16 走既有
  `LineIndex::extend`；UTF-16 新增 `LineIndex::extend_u16`（偶对齐扫描 +
  1 字节跨块 carry，奇数块边界不丢码元），推进 `indexed_bytes`，到 EOF
  置 done 并把 file_size 钉到真实大小；返回 `IndexStep{indexed_bytes,
  indexed_lines, done}`；
- 部分索引下 `chunk_at` 天然只服务已索引行（前端对 ≥indexed 的行只画
  「待索引」占位、从不取数）；`try_append` 在索引未完成时直接 NoChange，
  防止半截索引被 tail 扩展搅乱。

## 2. shell：threaded doc_open

- `doc_open`：readonly 模式（>200MB）`open_deferred` 后注册会话、
  spawn `std::thread` 逐块 `index_step`（每步短暂持锁，GUI 取块可穿插），
  每块经 Channel 发 `IndexProgress{indexed_bytes, indexed_lines, done}`；
  文档关闭后线程把余量索完（zombie 有界、tick 被前端忽略，不做取消标记）；
  edit 模式（≤200MB）原地循环索完（≤0.12s，P3 口径），语义同 v1；
- `DocInfo.total_lines` 只读延迟打开时为 `null`，
  `line_count_estimate = size/32`（粗估，占位行数与滚动条长度，
  done tick 校正为真实值）；`IndexProgress` 加 `indexed_lines`。

## 3. 前端

- `onIndexProgress`：非 done tick 更新 `indexed`（占位分界），节流 120ms
  刷新活动文档列表；done tick 校正 `ds.totalLines`、刷新 + 状态栏 +
  检查器；mountList 对延迟打开把 indexed 初值设 0（原为估算总数）。

## 4. 验收

- core 单测：extend_u16 分块（奇数块）与 build_aligned 全量一致；
  open_deferred + step 循环与 open 的行数/chunk_at 抽样一致；
  部分索引越界行报 LineOutOfRange；半截索引 try_append = NoChange；
- 前端 tsc/npm test/build 绿；Rust workspace + clippy 净；
- 人工：1GB 日志打开即首屏、占位行滚动推进（`tauri dev`）。
