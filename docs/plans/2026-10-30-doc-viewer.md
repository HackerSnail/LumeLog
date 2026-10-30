# 文档查看器 增量1：xlsx + docx Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 工具栏「查看」打开 xlsx（表格 + CSV 导出）与 docx（正文文本 + TXT 导出），纯 Rust 解析，复用 ParserPanel。

**Architecture:** 新 `app/src-tauri/src/viewers.rs`（`ViewerSessions` 会话 + 窗口取行 + CSV/TXT 导出）+ 4 命令；前端复用 `ParserPanel/RowStore` 建 `#viewer-dock`（「加载后续 1000 行」分窗拉取）。

**Tech Stack:** `calamine 0.37`（chrono feature）、`zip 9`（deflate）、`quick-xml 0.38`；dev-dep `rust_xlsxwriter`（写 fixture）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `npm run build`。

**Spec:** `docs/specs/2026-10-30-doc-viewer-design.md`。分支基：`main`（`6faa5da`）→ `feat/doc-viewer`。

## Global Constraints

- 会话上限 4（超限 `invalid_argument`）；文件 ≤64MB；docx 单条目解压 ≤64MB、段落 ≤20 万/64MB 文本（超限截断 + 标记）。
- xlsx 列上限 64（尾空列裁掉）；窗口 limit ≤1000；导出上限 1,048,576 行，CSV 带 UTF-8 BOM。
- 错误码只新增 `viewer_unsupported`；其余复用 io/not_found/invalid_argument/unknown_request。
- 逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. calamine `Data` 各变体的字符串化（浮点去尾零、DateTime/ ISO 变体、Error、Empty）。
2. docx XML：跨 `w:r` 的 `w:t` 拼接、`xml:space="preserve"`、无 document.xml 的容错。
3. CSV 与表格同源（同一 cells 映射）+ 引号转义；导出后 tmp→rename。
4. 会话生命周期：超限报错文案、`viewer_close` 幂等、未知 id。
5. 前端 token：切 sheet/关 dock 后旧 `viewer_rows` 回包丢弃。

---

## Task 1: Rust 查看器会话（`viewers.rs`）

**Files:** Create `app/src-tauri/src/viewers.rs`；Modify `app/src-tauri/Cargo.toml`、workspace `Cargo.toml`
**Interfaces:** `ViewerSessions::open(path) -> ViewerInfo{id,kind,sheets,size_bytes}`、`rows(id, sheet_index, from_row, limit) -> ViewerPage{total,columns,rows}`、`close(id)`、`export(id, sheet_index, target) -> ExportStats`；`ViewerKind::{Xlsx,Docx}`
- [ ] **Step 1：测试**：fixture 写 xlsx（多 sheet、数字/日期/布尔/空/长文本）+ docx（多段多 run）：open 信息、xlsx 窗口行 + 列字母 + 格式化、docx 段落行、CSV 导出同源 + 引号转义 + BOM、TXT 导出、未知扩展名 `viewer_unsupported`、会话上限 4、close 幂等、未知 id。
- [ ] **Step 2：** FAIL → 实现 → PASS；workspace 门。
- [ ] **Step 3：** CHANGELOG；commit `feat:viewer-sessions` + push。

---

## Task 2: 命令 + 注册 + 前端桥

**Files:** Modify `app/src-tauri/src/commands.rs`、`lib.rs`、`app/frontend/src/ipc.ts`、`ipc.test.ts`
**Interfaces:** `viewer_open/viewer_rows/viewer_close/viewer_export_csv`；前端 `viewerOpen/viewerRows/viewerClose/viewerExportCsv` + 类型；错误码 `viewer_unsupported`。
- [ ] **Step 1：测试**：typeof +1、新错误码中文文案。
- [ ] **Step 2：** FAIL → 实现 → PASS；`npm run build` + workspace 门。
- [ ] **Step 3：** CHANGELOG；commit `feat:viewer-ipc` + push。

---

## Task 3: 前端 dock（`main.ts` + 样式）

**Files:** Modify `app/frontend/src/main.ts`、`styles.css`
**Interfaces:** 工具栏「查看」按钮、`buildViewerDock()`（sheet 下拉 + ParserPanel 宿主 + 加载更多 + 导出）、`openViewerFile()/closeViewerDock()`（四 dock 互斥）、`viewerToken`。
- [ ] **Step 1：** 实现；`npm test` + `npm run build` 回归。
- [ ] **Step 2：** CHANGELOG；commit `feat:viewer-dock` + push。

---

## Task 4: 文档收尾

**Files:** Modify `docs/features.md`（条目注记：xlsx/docx 已交付，PDF/PPT 推迟原因）、`CHANGELOG.md`
- [ ] **Step 1：** 文档更新。
- [ ] **Step 2：** CHANGELOG；commit `docs:viewer-delivery` + push；`--no-ff` 合 `main` + push。
