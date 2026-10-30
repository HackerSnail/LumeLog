# 文档查看器 增量1：xlsx 表格 + docx 文本 设计（Plan 19）

对应 `docs/features.md` 待做第 2 条：「常见文档格式查看与导出（PDF、PPT、docx/xlsx 等 Office 格式）」。

## 1. 范围裁剪（明确不做与原因）

| 格式 | 本增量 | 原因 |
|---|---|---|
| **xlsx/xlsm** | ✅ 表格查看 + CSV 导出 | `calamine`（纯 Rust）读，依赖轻 |
| **docx** | ✅ 正文文本提取查看 + TXT 导出 | zip + `quick-xml` 提取 `w:p/w:t`，纯 Rust |
| PDF | ❌ 推迟 | 查看需 pdfium 级渲染引擎（原生库 ~5MB+、需随包分发）；纯文本提取版观感差。等真实需求再立项 |
| PPT | ❌ 推迟 | 版式复杂，文本提取价值低 |
| xls（老二进制）/ ods | ❌ 推迟 | calamine 支持但测试面翻倍；xlsx 已覆盖主流 |

插件接口增量3（Wasm host）同步推迟：需引入 wasmtime 级依赖，与「轻量」定位冲突，且当前
JS 沙箱插件已覆盖高亮与逐行解析，等出现 JS 表达不了的解析需求再立项。

## 2. 会话模型（照 MF4 前例）

`app/src-tauri/src/viewers.rs`：

- `ViewerSessions`：id → 会话，上限 4，超限 `invalid_argument`（提示先关闭）。
- `viewer_open(path) -> ViewerInfo`：按扩展名分流；`.xlsx/.xlsm` → `calamine::Xlsx`
  （`BufReader<File>` 常驻会话），返回 sheet 名列表；`.docx` → 打开 zip 提取
  `word/document.xml`（单条目解压上限 64MB，防 zip 炸弹）→ 按 `w:p` 切段落、段内拼接 `w:t`
  → 段落文本一次性驻留（上限 20 万段 / 64MB 文本，超限截断并标记）。文件 ≤64MB。
- `viewer_rows(id, sheet_index, from_row, limit) -> ViewerPage`：
  - xlsx：`worksheet_range` 物化该 sheet 后按行窗口跳取（from/limit ≤1000）；
    列 = 表宽（上限 64 列，尾部空列裁掉），单元格→字符串：`String` 原样、整数浮点去尾零、
    `Bool` → TRUE/FALSE、日期时间 → `YYYY-MM-DD HH:MM:SS`（calamine `chrono` feature）、
    空 → ""；返回列字母表（A/B/C…）作表头。
  - docx：单一"sheet"，行 = 段落，列 = 「内容」。
  - `total` = 全行数（xlsx 用 Range 尺寸，docx 用段落数），前端据此做「加载更多」。
- `viewer_close(id)`：幂等；`viewer_export_csv(id, sheet_index, target)`：
  全表流式写 CSV（500 行/批缓冲、RFC 引号转义、UTF-8 BOM 便于 Excel 直开），行数上限 1,048,576
  （xlsx 单表上限）；docx 导出为 TXT（段落换行）。tmp→rename。
- 新错误码：`viewer_unsupported`（扩展名不支持）。

## 3. 前端（复用增量2 的 ParserPanel）

- 工具栏「查看」→ 打开对话框（filter: xlsx/xlsm/docx）→ `viewer_open` → `#viewer-dock`
  （与报文/测量/解析三向互斥，同 `#can-dock` 高度契约）：sheet 下拉 + `ParserPanel`
  + 「加载后续 1000 行」 + 「导出 CSV/TXT」。
- 行数据 `ParserRow{line, cells}` 与解析面板同构，`RowStore` 直接累积；docx 无行号跳转
  （`onRowActivate` 不接线）。
- `viewer_rows` 按窗口拉取，token 丢弃旧响应。

## 4. 验收

- Rust：fixture 用 `rust_xlsxwriter`（dev-dep）现写 xlsx、`zip` 现写 docx——
  开关 sheet/窗口取行/数字与日期格式化/空单元格、docx 段落提取（跨 run 拼接）、CSV 导出与
  表格同源 + 引号转义、未知扩展名、会话上限 4、docx 超限截断标记；
  `cargo test --workspace` + `clippy -D warnings` 净。
- 前端：`npm test` + `npm run build` 绿（ipc typeof +1；面板/表格复用已测组件）。
- 人工：`npm run tauri dev` 打开真实 xlsx/docx 过一遍。
