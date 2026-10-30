# 语法高亮语言扩展 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把语法高亮从 5 种扩到 20 种常见编程/配置语言，模式 id 与扩展名解耦，语言包按需动态加载不撑大首屏。

**Architecture:** `language.ts` 改为注册表（`mode id → exts/label/loader`）+ `modeIdForExt`（同步纯函数）+ `resolveLanguage`（动态 import + 缓存）；`editor.ts` 的 `setMode` 收模式 id 并异步重配（token 防竞态）；`main.ts applyMode` 不再把模式名当扩展名传；`statusbar.ts` 用显示名渲染下拉。

**Tech Stack:** TS + CodeMirror 6；新增 `@codemirror/lang-javascript|python|html|css|xml|markdown|sql|rust|java`、`@codemirror/legacy-modes`。门：`npm test` / `npm run build`；Rust 侧无改动（`cargo test --workspace` + `clippy -D warnings` 仅确认不回归）。

**Spec:** `docs/specs/2026-10-30-syntax-languages-design.md`。分支基：`main`（`41985c3`）→ `feat/syntax-languages`。

## Global Constraints

- 现有模式 id（`auto`/`plain`/`log`/`ini`/`json`/`c`）与行为不变，`AUTO` 仍按扩展名推断。
- 语言包只能动态加载；首屏主 chunk 体积不增长（基线 623.3KB，见 §验收）。
- `log`/`ini` 继续用本地 `StreamLanguage` + `streamHighlight`；其余语言靠 `defaultHighlightStyle`（fallback）着色，不新增 `HighlightStyle`。
- 未知扩展名 → `null` = 纯文本，不得报错。
- 逐任务 TDD；门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. **注册表漂移**：`AVAILABLE_MODES` 每项都能解析出语言（测试覆盖全量）。
2. **竞态**：连续切语法时旧 `import()` 回包不得覆盖新语言（token）。
3. **异步接线**：`openEditable` 首帧语言已就位；`destroy()` 后回包不再 dispatch。
4. **体积**：主 chunk 不涨，语言成独立 chunk；`tsc --noEmit` 净。
5. **状态栏**：显示名与回调值分离后，`onMode` 仍收到模式 id。

---

## Task 1: 依赖 + 注册表（`language.ts` 重构）

**Files:** Modify `app/frontend/package.json`、`package-lock.json`、`app/frontend/src/language.ts`；Create `app/frontend/src/language.test.ts`
**Interfaces:** `export const AVAILABLE_MODES`（20 项）、`export type ModeChoice`、`export const MODE_LABELS: Record<ModeChoice,string>`、`export function modeIdForExt(ext: string): ModeChoice | null`、`export async function resolveLanguage(mode: ModeChoice | null): Promise<LanguageSupport | null>`
- [ ] **Step 1：** 装 10 个依赖（`npm i`），确认 `@codemirror/legacy-modes` 子路径 `mode/yaml` 等可直接 import（看 `node_modules/@codemirror/legacy-modes/package.json` 的 `exports`）。
- [ ] **Step 2：测试**：`ext→mode` 映射表（py/ts/tsx/yml/rs/md/sql/html/css/xml/java/js/jsx/sh/toml/diff/ps1/ini/log/json/c/hpp/未知）、`AVAILABLE_MODES` 除 auto/plain 外全部可解析、`resolveLanguage(null|"plain")===null`、二次调用同一实例。
- [ ] **Step 3：** FAIL → 实现（注册表 + 动态 import + `Map` 缓存；log/ini 同步）→ PASS。
- [ ] **Step 4：** CHANGELOG；commit `feat:syntax-registry` + push。

---

## Task 2: 语言落地（官方 lezer + legacy 流模式）

**Files:** Modify `app/frontend/src/language.ts`、`language.test.ts`
**Interfaces:** 见 spec §3.4 两表；`c` 扩 `hpp/cxx/hh`。
- [ ] **Step 1：测试**：`resolveLanguage("typescript")` 解析器识别 `interface`（按 `language.name`/`parser` 非空断言即可）、yaml/toml/shell/diff/powershell 解析出非空 `LanguageSupport`、扩展名补测 `hpp`、`mts`、`psd1`、`svg`。
- [ ] **Step 2：** FAIL → 实现（`javascript({typescript, jsx})` 按 id 取参数；legacy 模式 `StreamLanguage.define(x)`）→ PASS。
- [ ] **Step 3：** CHANGELOG；commit `feat:syntax-languages` + push。

---

## Task 3: `editor.ts` + `main.ts` 异步接线

**Files:** Modify `app/frontend/src/editor.ts`、`app/frontend/src/main.ts`
**Interfaces:** `EditorHandle.setMode(mode: ModeChoice | null): void`（内部异步 + token）；`openEditable` 先 `await resolveLanguage(modeIdForExt(opts.ext))`；`applyMode` 改为传模式 id。
- [ ] **Step 1：** 改 `editor.ts`：`buildState(text, lang, langConf, onDocChanged)` 接已解析语言；`setMode` 内 `++modeToken` 守卫，`destroy` 后回包丢弃。
- [ ] **Step 2：** 改 `main.ts applyMode`：`auto` → `modeIdForExt(extOf(ds.path))`，`plain` → null。
- [ ] **Step 3：** `npm test` + `npm run build` 绿；CHANGELOG；commit `feat:syntax-editor-wiring` + push。

---

## Task 4: 状态栏显示名 + 收尾

**Files:** Modify `app/frontend/src/statusbar.ts`、`statusbar.test.ts`、`docs/features.md`、`CHANGELOG.md`
**Interfaces:** `mkSelect(cls, labels, values?)`；`statusbar` 下拉显示 `MODE_LABELS`，`onMode` 回传模式 id。
- [ ] **Step 1：测试**（+1）：下拉选项文本为显示名、`change` 回传 id。
- [ ] **Step 2：** FAIL → 实现 → PASS。
- [ ] **Step 3：** 体积对比（`dist/index-*.js` 主 chunk 不涨）+ `cargo test --workspace` / `clippy` 不回归；`features.md` 语法条目移到已交付并写交付细节；CHANGELOG；commit `feat:syntax-statusbar-labels` + push；`--no-ff` 合 `main` + push。
