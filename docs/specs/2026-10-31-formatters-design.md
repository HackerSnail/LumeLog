# JSON/XML/SQL 格式化（Plan 42）

N++ JSON Viewer / XML Tools / VS Code 内置能力的对齐。作用于编辑器选区
（空选区 = 整篇），单次 dispatch 可一步撤销；失败 toast 校验错误不改文本。

## 1. 纯函数 `formatters.ts`（可测）

- `formatJson` / `minifyJson`：`JSON.parse` 校验 + `stringify(…, null, 2)`；
  解析错误原样透传（V8 消息自带位置）。已知限制：大整数精度（>2^53）
  经 roundtrip 会失真，典型日志 JSON 无此量级；
- `formatXml` / `validateXml`：`DOMParser`（application/xml）解析，
  parsererror → 失败；格式化按子树重排（元素缩进 2 空格、仅文本子元素
  内联、属性重新转义、注释/CDATA/PI 保留）。已知限制：doctype 内部子集
  与原文空白细节不保真；
- `formatSql`：词法级排版（永不失败）——扫描切分 '…' 字符串与 `--`、
  `/*…*/` 注释（内容原样），外部关键字统一大写、空白折叠，主子句
  （SELECT/FROM/WHERE/GROUP BY/ORDER BY/HAVING/LIMIT/OFFSET/VALUES/
  各类 JOIN/UNION/SET）前换行 + 2 空格缩进。best-effort，方言细节不保证。

## 2. 命令

palette 四条：「JSON：格式化」「JSON：压缩」「XML：格式化」「SQL：排版」，
替换经单次 dispatch（选区或整篇），失败 toast 错误不改内容。

## 3. 验收

- `formatters.test.ts`：JSON 格式化/压缩/坏输入、XML 属性转义/注释/
  自闭合/校验失败、SQL 关键字大写但字符串/注释原样、子句换行；
- tsc/npm test/build 绿；人工：对真实日志 JSON 选区操作（`tauri dev`）。
