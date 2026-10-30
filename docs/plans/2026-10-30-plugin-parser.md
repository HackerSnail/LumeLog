# 插件接口 增量2：QuickJS 数据解析插件 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `plugins/*.js` 逐行解析插件：rquickjs 沙箱执行 `parseLine`，结果流式发到新「解析」dock 的虚拟表格，点行跳源行。

**Architecture:** `plugins.rs` 扩展（元数据加载/校验 + `run_parser_plugin` 流式执行）+ `parser_plugins_list/parser_run/parser_cancel` 三命令（can_parse 双通道模式）；前端 `parserpanel.ts`（canpanel 骨架裁剪：动态列 + 自写虚拟滚动）+ `main.ts` dock 互斥接线。

**Tech Stack:** rquickjs 0.14（复用 script.rs 运行时模式）、@tanstack/table-core（已有依赖）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `npm run build`。

**Spec:** `docs/specs/2026-10-30-plugin-parser-design.md`。分支基：`main`（`454a5ee`）→ `feat/plugin-parser`。

## Global Constraints

- 沙箱同脚本引擎：64MiB/1MiB 栈/无 fs/net；元数据加载超时 5s、run 总超时 300s。
- 批量 512 行/帧；`begin_search/end_search` 取消注册表复用；前端不传插件路径（后端按 id 扫描）。
- id 与语法插件共用命名空间与规则（1–64 `[A-Za-z0-9_-]`）；文件名序、first wins。
- cells 全字符串化（number/bool 转换，null/undefined→""，其它类型→"" + console warning）。
- 逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. rquickjs 值转换的 panic 面（Object get / to string 全部走 Result，不 unwrap 用户数据）。
2. 取消与超时：interrupt handler + 逐行检查；`on_batch.send` 失败后排空继续。
3. 前端虚拟滚动：ROW_H 常量、rAF 节流、destroy 后 scroll 不再 paint。
4. dock 互斥：打开解析 dock 关 CAN/MF4，反之亦然；closeParser 释放 token/cancel。
5. 错误码两侧一致（`plugin_not_found`/`plugin_bad_manifest` 进前端表 + 测试断言）。

---

## Task 1: Rust 元数据加载（`plugins.rs` + `parser_plugins_list`）

**Files:** Modify `app/src-tauri/src/plugins.rs`、`app/src-tauri/src/lib.rs`
**Interfaces:** `ParserColumnJson{key,label}`、`ParserPluginInfo{id,name,extensions,columns}`、`ParserPluginsReply{parsers,errors}`、`list_parser_plugins(dir) -> (Vec<ParserPluginInfo>, Vec<String>)`、`load_parser_metadata(path) -> Result<ParserPluginInfo, String>`（rquickjs 执行 + 读 `plugin`）、`#[tauri::command] parser_plugins_list`
- [ ] **Step 1：测试**：合法插件元数据、缺 `plugin` 对象、`parseLine` 非 function、columns 空/超 32、key 非法字符、坏 JS 报错不阻塞其它、id 重复 first wins、`.json` 不入解析器列表。
- [ ] **Step 2：** FAIL → 实现（`Runtime::new` + `set_memory_limit/set_max_stack_size` + 5s interrupt handler；`ctx.eval` 后 `global.get("plugin")`，`const` 回退 eval 读取仿 script.rs L368-378）→ PASS；workspace 门。
- [ ] **Step 3：** CHANGELOG；commit `feat:parser-plugin-metadata` + push。

---

## Task 2: 流式执行 + 命令（`run_parser_plugin` + `parser_run/parser_cancel`）

**Files:** Modify `app/src-tauri/src/plugins.rs`、`app/src-tauri/src/commands.rs`、`lib.rs`
**Interfaces:** `ParserRowJson{line:u32,cells:Vec<String>}`、`ParserStats{rows,lines,skipped,cancelled,error:Option<String>}`、`run_parser_plugin(doc:&Arc<Mutex<DocFile>>, plugin_path:&Path, on_batch, on_console, cancel) -> ParserStats`、`#[tauri::command] parser_run(state,app,doc_id,plugin_id,on_batch,on_done) -> u64`、`parser_cancel(request_id)`
- [ ] **Step 1：测试**：端到端（临时 doc+插件：行→cells 顺序与字符串化、null 跳过计数、批 ≤512）、cancel 中途停止（cancelled=true）、id 不存在 → `plugin_not_found`、未知 doc → `unknown_doc`、console 透传。
- [ ] **Step 2：** FAIL → 实现（DocLines 逐行；`begin_search`/`end_search`；`spawn_blocking`+`catch_unwind` 仿 script_run）→ PASS；workspace 门。
- [ ] **Step 3：** CHANGELOG；commit `feat:parser-plugin-run` + push。

---

## Task 3: 前端桥（`ipc.ts` + 错误码）

**Files:** Modify `app/frontend/src/ipc.ts`、`app/frontend/src/ipc.test.ts`
**Interfaces:** `ParserPluginInfo/ParserRowJson/ParserStats/ParserPluginsReply`、`parserPluginsList()`、`parserRun({docId,pluginId},onBatch,onDone) -> Promise<number>`、`parserCancel(id)`；错误码 `plugin_not_found`/`plugin_bad_manifest`。
- [ ] **Step 1：测试**：typeof +1、新错误码中文文案非空。
- [ ] **Step 2：** FAIL → 实现（双通道照 canParse）→ PASS。
- [ ] **Step 3：** CHANGELOG；commit `feat:parser-plugin-ipc` + push。

---

## Task 4: `ParserPanel`（新 `parserpanel.ts`）

**Files:** Create `app/frontend/src/parserpanel.ts`、`parserpanel.test.ts`
**Interfaces:** `new ParserPanel(host, {onRowActivate, onError})`、`setColumns(cols: {key,label}[])`、`push(rows: ParserRowJson[])`、`setDone(stats)`、`clear()`、`get rowCount()`、`destroy()`
- [ ] **Step 1：jsdom 测试**：setColumns 后列头渲染、push 累积行且 rowCount 正确、点击行回传行号、clear 清空、destroy 后不再响应。
- [ ] **Step 2：** FAIL → 实现（table-core 动态 ColumnDef、虚拟滚动镜像 canpanel：ROW_HEIGHT+spacer+可视窗+rAF；无过滤）→ PASS；`npm run build`。
- [ ] **Step 3：** CHANGELOG；commit `feat:parser-panel` + push。

---

## Task 5: dock 接线（`main.ts`）

**Files:** Modify `app/frontend/src/main.ts`
**Interfaces:** 工具栏「解析」按钮、`buildParserDock()`（插件下拉 + 解析/停止 + 面板宿主）、`toggleParserDock()/closeParser()`（与 CAN/MF4 互斥）、`parserToken`、行点击 `revealSourceLine`、文档关闭时若 dock 开启则关闭。
- [ ] **Step 1：** 实现 + 手动冒烟点（npm test 全量回归 + build）。
- [ ] **Step 2：** CHANGELOG；commit `feat:parser-dock` + push。

---

## Task 6: 文档收尾

**Files:** Modify `docs/features.md`（待做第 1 条注记更新：增量2 已交付、剩余 Wasm host）、`CHANGELOG.md`
- [ ] **Step 1：** 文档更新。
- [ ] **Step 2：** CHANGELOG；commit `docs:plugin-parser-delivery` + push；`--no-ff` 合 `main` + push。
