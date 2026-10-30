# PDF 文本搜索 增量3 设计（Plan 24）

真渲染（Plan 22）的配套增量：在查看器 dock 的 PDF 视图内做**全文档文本搜索 + 命中高亮 + 跳转**，
搜索框沿用查询面板的交互习惯（输入 + 计数 + 上/下一条 + Esc 清除）。

## 1. 方案

**自研轻量搜索，不用 pdf.js 的 `PDFFindController`。** 后者绑死 `PDFViewer` 全套组件（会替换掉
现有 canvas 渲染路径与 dock 结构），且带 findBar UI；我们只需要「找文本 → 算矩形 → 画高亮」，
`page.getTextContent()` + 几何换算就够，代码量 1/5。

- 每页 `getTextContent()` 结果缓存（`Map<page, items>`，items 含 `str/transform/width/height`）；
- **匹配**：查询串在单个 text item 内做大小写不敏感匹配（跨 item 的词不识别——PDF 文本 item
  切分由生成器决定，跨 item 匹配的正确矩形无法按字符比例近似，明确为已知限制）；
- **矩形**：`pdfjs.Util.transform(viewport.transform, item.transform)` 得设备坐标，命中框按
  字符在 item 内的比例横向切分（`charW = itemWidth / str.length`）——等宽近似，CJK 与常规
  西文足够，斜体/字距紧排会有偏差（已知近似，spec §3）；
- **高亮**：canvas 外叠一层 `.pdf-hits`（绝对定位 div，当前命中 `pdf-hit-current` 强调色）；
  每次渲染后按当前 viewport 重算矩形（缩放/翻页后命中框跟随）；
- **跳转**：`gotoHit(i)` 切页渲染并滚动到命中框；`Esc`/清空按钮清除全部命中。

## 2. API（`pdfview.ts` 扩展）

```
searchAll(query: string) -> Promise<number>     // 全文档（逐页缓存 text content），返回命中数
gotoHit(index: number) -> Promise<void>          // 切页渲染 + 画高亮 + 滚动定位
clearSearch() -> void
// 纯函数（可测）：
matchItem(itemStr, query) -> number[]            // 命中起始字符下标列表
hitRect(item, viewportTransform, dpr, charStart, charLen) -> {left, top, width, height} | null
```

- `searchAll` 逐页进行，每页完成后经 `onSearchProgress(done, total)` 回调（大 PDF 不假死）；
- 翻页/缩放保留命中集合，渲染后重画当前页命中；切换文档/清空时重置。

## 3. 已知限制（写进 features.md 注记）

跨 item 匹配不识别；命中框为字符比例近似（无字形度量）；RTL/竖排不处理；加密 PDF 同现状报错。

## 4. 验收

- 纯函数单测：`matchItem` 大小写不敏感/重叠命中/空查询、`hitRect` 等宽比例切分与 dpr 换算；
- `tsc --noEmit` + `npm test` + `npm run build` 绿（Rust 侧零改动）；
- 人工：真实 PDF 搜索英文/中文关键词，命中高亮跟随缩放与翻页。
