# Find in Files：目录级多文件搜索（Plan 44）

N++ 目录搜索对齐。扫描逻辑进 lumelog-core（编码检测/解码/regex 复用），
app 包命令（复用 filter_run 的 req_id + 双 Channel + 取消注册表模式），
前端一个自持面板（目录选择 + 字面/正则 + 大小写 + 结果列表）。

## 1. core：`fif.rs`

- `FifMatcher::build(pattern, mode, case)`：literal（大小写不敏感走
  to_lowercase 包含）或 `regex::Regex`（编译错误透传）；
- `scan_dir(root, matcher, max_hits=10_000, max_file_bytes=64MiB, on_batch,
  cancelled) -> FifStats{files, skipped, hits, truncated}`：
  - 递归走目录（栈实现），跳过 `.` 开头目录与 `.git/target/node_modules`、
    跳过符号链接、跳过 >64MiB 文件（计 skipped）；
  - 每文件：头 64KB `encoding::detect` → `decode_all` 整体解码 → 逐行
    匹配（命中行截断 500 字符）；批满 200 条经回调发出；
  - 全局命中上限后 `truncated = true` 收敛；cancelled 每文件检查；
- 命中路径输出相对 root（`\` 归一为 `/`）。

## 2. shell 命令

`fif_run(root, pattern, mode, case_sensitive, on_batch, on_done) -> req_id`
（spawn_blocking + begin/end_search 取消注册表，照 filter_run 模式）、
`fif_cancel(request_id)`。root 非目录 → `invalid_argument`。

## 3. 前端

- palette「在文件中查找…」开自持面板：目录输入 + 浏览（dialog
  directory）、查找串、字面/正则、大小写、查找/停止、状态行
  （N 文件 · M 命中 · 截断提示）、结果列表（命中行 `相对路径:行号: 文本`，
  点击 openPath 后按路径定位——编辑器 `revealLine(line)`，只读列表
  `revealLine(line - 1)`）；
- 渲染上限 1000 行（超出提示收窄查找范围）。

## 4. 验收

- core 单测：literal/regex/大小写、GBK 文件命中、截断、大文件跳过、
  点目录跳过、cancelled 收敛；
- Rust workspace + clippy 净；tsc/npm test/build 绿；
- 人工：目录搜索→点命中→定位（`tauri dev`）。
