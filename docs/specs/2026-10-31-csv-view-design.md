# CSV/分隔日志列表视图（Plan 45）

EmEditor 式的"按分隔符切列进表格"。复用解析 dock 全套基建
（ParserPanel/RowStore/虚拟表格/行号回跳/取消注册表），给解析插件下拉
加一个内置项「分隔符表格」，走新命令 `csv_run` 流式解析当前文档。

## 1. core：`csv.rs`

- `split_line(line, delim)`：单字符分隔；双引号段内分隔符失效、`""`
  为字面引号（RFC 4180 子集）；末字段去尾部 `\r`。纯函数可测。

## 2. shell：`csv_run`

- `csv_run(doc_id, delimiter, header, on_batch, on_done) -> req_id`：
  DocLines 流式（同 parser_run），首行作表头时第一批带 `columns`，
  其余批只带行；列上限 64（超出的字段并入第 64 列，数据不丢），
  行上限 50 万；空行计 skipped；取消共用 begin/end_search；
- 批形状复用 `ParserRowJson{line, cells}`——前端表格/回跳零改动。

## 3. 前端

- 解析 dock：插件下拉首项「内置：分隔符表格（CSV/TSV）」，选中时显示
  分隔符（`,`/Tab/`;`/`|`）与「首行为表头」控件；解析按钮分流到
  `csvRun`——首批到达后按 header 或行宽 setColumns（列可随批次增长），
  后续批 push 渲染；停止沿用 parserCancel（同一注册表）。

## 4. 验收

- core 单测：普通切分、引号段内分隔符/`""`、尾部 `\r`、连续分隔符空字段；
- Rust workspace + clippy 净；tsc/npm test/build 绿；
- 人工：CSV/TSV 日志进表格、点行回跳（`tauri dev`）。
