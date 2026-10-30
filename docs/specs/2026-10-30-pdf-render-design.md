# PDF 真渲染 增量2 设计（Plan 22）

> 依据立项文档 `2026-10-30-pdf-viewer-proposal.md`：真渲染选 **路线 C（pdf.js）**——无原生库
> 分发成本、体积约 1/3、Firefox 同款引擎。覆盖 A（pdfium）的决策点仅在 webview 渲染性能
> 成为实际反馈时重开。增量1（文本提取）已交付，TXT 导出保留。

## 1. 架构

- **渲染在前端**（渲染本属前端职责）：`pdfjs-dist` 动态 `import()`，主包零增长；
  worker 用 `pdfjs-dist/build/pdf.worker.min.mjs?url`（Vite `?url` 产出独立 chunk）。
- **后端只出字节**：新命令 `viewer_pdf_bytes(id) -> tauri::ipc::Response`（原始字节，非 JSON 数组，
  64MB 上限内不膨胀 33%）；pdf 会话在 `viewer_open` 时只存路径（**不再在打开时提取文本**——
  提取失败的 PDF 仍可正常渲染），文本提取改为 TXT 导出与 `rows()` 时的按需操作（catch_unwind 兜底）。
- **PDF 的字节数据经 invoke 直接进 pdf.js `getDocument({data})`**，不落临时文件。

## 2. 前端

- 新 `pdfview.ts`：`PdfView` 类——`load(data: ArrayBuffer) -> numPages`、
  `render(page1, scale)`（canvas 渲染，DPR 缩放、rAF 去重、渲染任务 cancel 防翻页竞态）、
  `destroy()`；纯函数 `clampPage`/`stepZoom`（范围 0.5–4，步进 1.25 倍）单独导出供 jsdom 测试。
  pdf.js 的 DOM/worker 依赖进不了 jsdom，组件级只测错误路径与纯函数。
- `main.ts` 查看器 dock 按 `kind === "pdf"` 切换：隐藏 sheet 下拉/表格/加载更多，
  显示 PDF 工具条（上一页/页码输入/共 N 页/缩放下拉 0.5–4/适应宽度）+ canvas 宿主；
  「导出 CSV」按钮在 pdf 下文案变「导出 TXT」。
- worker 与主线程通信失败（CSP/asset 协议）时回退 fake worker 警告并继续（pdf.js 自带降级）。

## 3. 不做（后续增量）

- 文本选择/搜索（需 pdf.js textLayer + 命中高亮，单独增量）；缩略图侧栏；旋转；连续滚动模式；
  打印。加密 PDF 维持打开时报错。

## 4. 验收

- Rust：`viewer_pdf_bytes` 返回与文件一致的字节；pdf 会话 open 不再提取（畸形 PDF open 成功、
  导出/rows 才报错）；`rows()`/`export()` 按需提取路径回归；workspace test + clippy 净。
- 前端：`clampPage`/`stepZoom` 单测；`tsc --noEmit` + `npm test` + `npm run build` 绿；
  主 chunk 体积不涨（pdfjs 落独立 chunk，记入 CHANGELOG）。
- 人工：`tauri dev` 打开多页 PDF 翻页/缩放；损坏 PDF 出中文错误。
