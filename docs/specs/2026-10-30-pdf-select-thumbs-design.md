# PDF 文本选择 + 缩略图 增量4 设计（Plan 26）

真渲染（Plan 22）/搜索（Plan 24）的配套增量，Rust 侧零改动。

## 1. 文本选择（pdf.js TextLayer）

- 每页渲染完成后，在 canvas 上叠一层 `TextLayer`（pdfjs-dist 6 导出：
  `new TextLayer({ textContentSource: page.getTextContent(), container, viewport })` → `.render()`；
  重复渲染时对新层重新构造并移除旧层，不做 in-place update（页码/缩放都会变））；
- TextLayer 的 viewport 用 **CSS 像素 scale（不含 dpr）**——canvas 是设备像素、CSS 尺寸缩放显示，
  文本层必须与 CSS 尺寸对齐；
- 该版本 TextLayer 不自动注入样式，`styles.css` 手写 `.textLayer` 最小样式（absolute、
  `color: transparent`、`::selection` 半透明、`user-select: text`、`pointer-events: auto`），
  覆盖层 `.pdf-hits` 的 `pointer-events:none` 不变，命中框压在文本层之上互不干扰；
- 翻页/缩放/destroy 时取消 in-flight 的 TextLayer render（`textLayer.render()` promise 忽略取消错误）；
- 复制/选择的收益即为白名单目标：不做注释/标注。

## 2. 缩略图栏

- PDF 模式下在 `.pdf-host` 左侧显示固定宽 96px 的竖排缩略图栏（按钮「缩略图」切换显隐，默认开）；
- 每页一个 96px 宽的 canvas（高度按首页纵横比推算，各页各自按自身比例）；渲染 scale =
  `thumbW / pageWidth@scale1`（纯函数 `thumbScale`，可测）；
- **懒渲染**：`IntersectionObserver` 进入视口才渲染（大 PDF 不一次性渲染上百页），渲染过的不重画；
- 点击缩略图 → `renderViewerPdf(page)`；当前页的缩略图加边框，翻页/跳转时同步；
- 文档切换/destroy 时取消未完成的渲染任务并清空。

## 3. 验收

- `thumbScale`/页纵横比纯函数单测；既有 pdfview 测试不回归（构造后 DOM 仍是 1 个 canvas）；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；Rust 零改动；
- 人工：`tauri dev` 中选择/复制 PDF 文本、缩略图懒加载与点击跳页。
