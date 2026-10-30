# PPTX 文本提取查看 设计（Plan 25）

> PPT 的**真渲染**（版式/主题/图片/动画）不做——工作量与维护成本远超价值，features.md 相关
> 条目以本增量收口：pptx 交付**文本提取查看**（与 docx 同构），.ppt 老二进制格式不支持。

## 1. 方案

完全沿用 Plan 19 的文本行路径（docx 同款）：

- `viewers.rs` 接受 `.pptx`：zip 内收集 `ppt/slides/slide<N>.xml`，**按 N 数值排序**（字典序会把
  slide10 排到 slide2 前），逐张提取 `<a:p>` 段落内的 `<a:t>` 文本（quick-xml，docx 同款解析器写法）；
- 每张幻灯片前插入一行 `── 幻灯片 N ──` 分隔标记；空幻灯片只有分隔行；
- 段落/文本总量沿用既有上限（20 万行 / 64MB，超限截断标记）；打开时一次性提取（幻灯片文件
  数量有限，docx 同级成本）；
- TXT 导出、查看 dock 单列文本、`kind="pptx"` 走非 CSV 导出分支——前端只改文件对话框 filter。

## 2. 不做

版式渲染、母版/主题、图片/表格/图表对象、备注页（`notesSlide`）、.ppt 老格式、幻灯片内跳转。

## 3. 验收

- fixture 用 zip 现造（slide1/2/10 + 空幻灯片 + 跨 run 文本）：数值排序、段落切分、分隔行、
  空幻灯片、TXT 导出同源；坏 pptx（无 slides 条目）报错不 panic；
- `cargo test --workspace` + `clippy` 净；前端 `npm test`/`build` 回归（filter 改动）。
