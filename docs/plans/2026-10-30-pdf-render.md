# PDF 真渲染 增量2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 查看器 dock 内 pdf.js 真渲染：翻页/页码/缩放，主包零增长；后端只出字节，文本提取改按需（导出/rows）。

**Architecture:** `viewers.rs` pdf 会话改「存路径、按需提取」+ `viewer_pdf_bytes` 二进制命令；新 `pdfview.ts`（pdfjs-dist 动态加载 + worker + DPR canvas）；`main.ts` dock 按 kind 切换表格/PDF 两态。

**Tech Stack:** `pdfjs-dist@5`（动态 import + `?url` worker）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `npm run build`。

**Spec:** `docs/specs/2026-10-30-pdf-render-design.md`。分支基：`main`（`b2b3cf1`）→ `feat/pdf-render`。

## Global Constraints

- pdfjs-dist 只能动态 import；主 chunk 不涨。
- 后端字节走 `tauri::ipc::Response`（非 JSON 数组）；会话上限/文件 ≤64MB 沿用。
- pdf open 不得因提取失败失败；提取只在导出/rows 时按需发生并 catch_unwind。
- 渲染竞态：翻页/缩放连点时旧 render task `cancel()`。
- 逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. pdf.js worker 在 Tauri 资产协议下的加载路径（`?url` 相对 chunk）；失败降级不白屏。
2. `viewer_pdf_bytes` 的前端 invoke 二进制返回类型断言；IpcError 透传。
3. DPR/缩放后 canvas 尺寸与绘制的清晰度；destroy 后回包不渲染。
4. 提取按需化后既有 docx/pdf 测试不回归（open 不再预提取）。

---

## Task 1: Rust 会话改造 + 字节命令

**Files:** Modify `app/src-tauri/src/viewers.rs`、`commands.rs`
**Interfaces:** 会话加 `pdf_path: Option<PathBuf>`；`open_pdf` 不提取；`rows()/export()` pdf 分支按需提取；`#[tauri::command] viewer_pdf_bytes(state, id) -> tauri::ipc::Response`
- [ ] **Step 1：测试**：pdf open 成功且无预提取（构造提取必失败的 PDF 也能 open）；`viewer_pdf_bytes`（经命令层测不了，直测会话存路径 + 字节命令薄层）；畸形 PDF 导出/rows 才报错。
- [ ] **Step 2：** FAIL → 实现 → PASS；workspace 门。
- [ ] **Step 3：** CHANGELOG；commit `feat:pdf-bytes` + push。

---

## Task 2: `pdfview.ts` + 前端桥

**Files:** Modify `app/frontend/package.json`（`pdfjs-dist`）；Create `app/frontend/src/pdfview.ts`、`pdfview.test.ts`；Modify `app/frontend/src/ipc.ts`（`viewerPdfBytes(id): Promise<ArrayBuffer>`，错误处理同 `call`）
**Interfaces:** `PdfView(host, {onError})`：`load(data): Promise<number>`、`render(page, scale): Promise<void>`、`destroy()`；纯函数 `clampPage(n,total)`、`stepZoom(zoom, dir)`（×1.25，clamp 0.5–4）
- [ ] **Step 1：测试**：`clampPage` 边界（0/负数/超尾）、`stepZoom` 上下限；load 前调用 render 不炸。
- [ ] **Step 2：** FAIL → 实现（动态 import、worker `?url`、DPR、`RenderTask.cancel`）→ PASS；`npm run build` 确认 pdfjs 落独立 chunk。
- [ ] **Step 3：** CHANGELOG；commit `feat:pdf-view` + push。

---

## Task 3: dock 两态切换（`main.ts` + 样式）

**Files:** Modify `app/frontend/src/main.ts`、`styles.css`
**Interfaces:** pdf 工具条（上一页/页码 input/共 N 页/缩放下拉/适应宽度）；`openViewerFile` kind=pdf 时 `viewerPdfBytes` → `pdfView.load`；sheet/表格/加载更多隐藏；导出按钮文案
- [ ] **Step 1：** 实现；`npm test` + `npm run build` 回归。
- [ ] **Step 2：** CHANGELOG；commit `feat:pdf-render-dock` + push；`--no-ff` 合 `main` + push。
