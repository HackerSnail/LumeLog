# 拖放打开 + 最近文件 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 偿还 M1 欠账：拖文件进窗口即打开（遮罩反馈），命令面板注入最近 20 文件（后端 `doc_open` 自动记录）。

**Architecture:** 新 `app/src-tauri/src/recent.rs`（`recent.json` 读写/记录/清空，`doc_open` 成功后挂钩）+ `recent_list/recent_clear` 命令；前端 `getCurrentWebview().onDragDropEvent` + 全屏遮罩 + palette 最近条目。

**Tech Stack:** 无新依赖（`@tauri-apps/api` 已含 webview drag-drop 事件）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `npm run build`。

**Spec:** `docs/specs/2026-10-30-drop-recent-design.md`。分支基：`main`（`f4791d6`）→ `feat/drop-recent`。

## Global Constraints

- 记录只在 `doc_open` 成功后（BLF/DBC/查看器不算）；上限 20、去重置顶、原子写（tmp→rename）。
- `recent_list` 过滤不存在路径；损坏 JSON → 空表不报错。
- 拖放顺序打开，busy 单飞沿用 `openPath`；非 Tauri 环境降级不监听。
- 逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. `recent.json` 并发写（doc_open 与 recent_clear 竞争）——写入走同一把锁？记录在 doc_open 内串行，clear 命令同为同步命令；用文件原子写 + 幂等语义即可，测试覆盖「clear 后再 record」。
2. 拖放事件 payload 类型（enter/over/drop/leave）；overlay 不挡输入（pointer-events:none）。
3. palette 条目 id 唯一、与 keymap 动作 id 空间不相交（`recent-<i>`）。
4. 中文文案；损坏 JSON 不 toast 轰炸。

---

## Task 1: Rust 最近文件存储（`recent.rs` + 命令 + doc_open 挂钩）

**Files:** Create `app/src-tauri/src/recent.rs`；Modify `app/src-tauri/src/commands.rs`（`recent_list/recent_clear` + `doc_open` 成功后 `record_recent`）、`lib.rs`
**Interfaces:** `recent_path(app)`、`load_recent(path) -> Vec<String>`、`record_recent(path, opened) -> std::io::Result<()>`、`clear_recent(path) -> std::io::Result<()>`；`RECENT_CAP=20`
- [ ] **Step 1：测试**：空/缺失文件 → 空；记录去重置顶；第 21 条挤掉最旧；损坏 JSON → 空；clear 后为空且可再记录；原子写无 tmp 残留。
- [ ] **Step 2：** FAIL → 实现 → PASS；workspace 门。
- [ ] **Step 3：** CHANGELOG；commit `feat:recent-store` + push。

---

## Task 2: 前端（拖放 + palette 最近条目）

**Files:** Modify `app/frontend/src/ipc.ts`、`ipc.test.ts`、`app/frontend/src/main.ts`、`styles.css`
**Interfaces:** `recentList()/recentClear()`；`initRecent()/refreshRecent()`、`initDragDrop()`（动态 import + `.drop-overlay` 遮罩）、`buildCommands()` 注入 `recent-<i>` 条目与「清除最近文件记录」、`openPath` 成功后 `refreshRecent`
- [ ] **Step 1：测试**：ipc typeof +2、新命令无新错误码（复用 io）。
- [ ] **Step 2：** 实现；`npm test` + `npm run build` 回归。
- [ ] **Step 3：** CHANGELOG；commit `feat:drop-recent-ui` + push；`--no-ff` 合 `main` + push。
