# 插件接口 增量1：声明式语法高亮插件 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `$APP_CONFIG/LumeLog/plugins/*.json` 声明式语法插件：Rust 校验加载 + 前端编译为 StreamLanguage，状态栏出现新模式、扩展名自动匹配，不改主程序代码即可补冷门语言高亮。

**Architecture:** 新 `app/src-tauri/src/plugins.rs`（读目录/校验/`syntax_plugins_list` 命令）；前端 `language.ts` 加插件注册表（`autoModeId` 内置优先、`resolveLanguage` 认 `plugin:<id>`），`statusbar.ts` 加 `setSyntaxModes()`，`main.ts` 启动加载 + 命令面板「重载语法插件」。

**Tech Stack:** 无新依赖（serde_json/toml 已有；前端复用 @codemirror/language StreamLanguage）。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `npm run build`。

**Spec:** `docs/specs/2026-10-30-plugin-syntax-design.md`。分支基：`main`（`14470df`）→ `feat/plugin-syntax`。

## Global Constraints

- 插件不执行任何代码；manifest 校验按 spec §2（上限、tag 枚举、`deny_unknown_fields`）。
- 单文件失败不阻塞其它；id 按**文件名排序**去重，后到者报错跳过。
- 内置扩展名优先于插件；插件模式 id 一律 `plugin:<id>`。
- 命令面板重载不影响已打开编辑器。
- 逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. 校验完整性：tag 枚举、上限、空白/字符集、未知字段；坏文件 errors 带文件名。
2. `autoModeId` 冲突方向（内置赢）；`plugin:<id>` 与内置 id 空间不相交。
3. StreamLanguage tokenizer：sticky regex 从流位置锚定、`eatSpace`/关键词词边界、`lineComment` 前缀。
4. 状态栏重建保持选中；`onMode` 回传 `plugin:<id>`。
5. 重载竞态：连续点击只做一次 in-flight 合并（复用 promise 缓存）。

---

## Task 1: Rust 插件仓库（`plugins.rs` + 命令）

**Files:** Create `app/src-tauri/src/plugins.rs`；Modify `app/src-tauri/src/commands.rs`（`syntax_plugins_list`）、`lib.rs`（mod + 注册）
**Interfaces:** `SyntaxRuleJson{regex,tag}`、`SyntaxPluginJson{id,name,extensions,keywords,line_comment,rules}`（serde camelCase）、`SyntaxPluginsReply{plugins,errors}`、`load_syntax_plugins(dir) -> (Vec<SyntaxPluginJson>, Vec<String>)`、`#[tauri::command] syntax_plugins_list(app) -> Result<SyntaxPluginsReply, IpcError>`
- [ ] **Step 1：测试**：合法 manifest 解析、非法 tag/未知字段/超限拒绝、坏 JSON 跳过且 errors 带文件名、id 重复后到者跳过、目录缺失返回空、文件数截断报错。
- [ ] **Step 2：** FAIL → 实现（`plugins_dir` 复用 `commands::config_path`；`read_dir` NotFound → 空）→ PASS；`cargo test --workspace` + `clippy -D warnings` 净。
- [ ] **Step 3：** CHANGELOG；commit `feat:plugin-syntax-store` + push。

---

## Task 2: 前端注册表（`language.ts` + `ipc.ts`）

**Files:** Modify `app/frontend/src/language.ts`、`app/frontend/src/ipc.ts`；Modify `app/frontend/src/language.test.ts`、`app/frontend/src/ipc.test.ts`（typeof +1）
**Interfaces:** `type ModeId = ModeChoice | \`plugin:${string}\``、`registerPluginLanguages(plugins): string[]`（返回 warning 文案）、`autoModeId(ext): ModeId | null`、`syntaxModes(): {id,label}[]`、`resolveLanguage(mode: string | null)`；`SyntaxPlugin/SyntaxPluginsReply/syntaxPluginsList()`。
- [ ] **Step 1：测试**：注册后 `resolveLanguage("plugin:plc")` 出 `LanguageSupport` 且可 parse；`autoModeId("plc")` → 插件模式、`autoModeId("py")` 仍内置（内置优先）；坏正则规则跳过且 warning 提及插件名；`syntaxModes()` 含内置 21 项 + 插件；`resolveLanguage` 对未知 `plugin:x` 返回 null。
- [ ] **Step 2：** FAIL → 实现（StreamLanguage：`lineComment` 前缀 → rules sticky 匹配 → keywords 词边界 → 默认吞一个非空白字符；正则 `new RegExp(src, "y")`）→ PASS。
- [ ] **Step 3：** CHANGELOG；commit `feat:plugin-syntax-registry` + push。

---

## Task 3: 接线（`statusbar.ts` + `editor.ts` + `main.ts`）

**Files:** Modify `app/frontend/src/statusbar.ts`（`setSyntaxModes(modes)` 重建下拉、`onMode` 放宽为 `(mode: string) => void`）、`app/frontend/src/editor.ts`（`setMode(mode: string | null)`；首语言 `autoModeId(opts.ext)`）、`app/frontend/src/main.ts`（`initSyntaxPlugins()` 启动加载 + in-flight 合并；`applyMode` 收 string；palette「重载语法插件」）；Modify `app/frontend/src/statusbar.test.ts`（+1）
- [ ] **Step 1：测试**：`setSyntaxModes` 重建选项（显示名/值分离）、保持当前选中、`change` 回传 `plugin:<id>`。
- [ ] **Step 2：** FAIL → 实现 → PASS；`npm test` + `npm run build` 绿。
- [ ] **Step 3：** CHANGELOG；commit `feat:plugin-syntax-wiring` + push。

---

## Task 4: 文档收尾

**Files:** Modify `docs/features.md`（待做第 1 条加 ↳ 增量1 交付注记）、`CHANGELOG.md`
- [ ] **Step 1：** 文档更新（features.md 注记含 manifest 示例路径与字段摘要）。
- [ ] **Step 2：** CHANGELOG；commit `docs:plugin-syntax-delivery` + push；`--no-ff` 合 `main` + push。
