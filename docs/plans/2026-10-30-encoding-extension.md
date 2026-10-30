# 编码扩展（EUC-KR / EUC-JP / KOI8-R）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `EncodingLabel` 加 `EucJp / EucKr / Koi8R / Koi8U`，解码/编码/检测/前端菜单全链路打通，features.md 该条交付并修正 Big5/Shift-JIS 笔误。

**Architecture:** 全部改动集中在 `encoding/mod.rs` 的 5 个函数 + `detect` 的无 hint 二次确认 + 前端 `encoding-menu.ts` 一张表 + `ipc.test.ts` 断言 + 三份文档；`bom_len`/`IncrementalDecoder`/`encode_all` 不动（无 BOM，legacy `?` 分支已覆盖）。

**Tech Stack:** `encoding_rs`（EUC_JP/EUC_KR/KOI8_R 均内置）+ `chardetng`（三者均可检出）；无新依赖。门：`cargo test --workspace` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `npm run build`。

**Spec:** `docs/specs/2026-10-30-encoding-extension-design.md`。分支基：`main`（`7a19809`）→ `feat/encoding-extension`。

## Global Constraints

- 显示名固定：`EUC-JP` / `EUC-KR` / `KOI8-R` / `KOI8-U`；`from_label` 另收 `EUCJP / EUCKR / KOI8R / KOI8U` 别名。
- 检测：仅新增「带 hint 判定为 GBK/GB18030 时用无 hint 复核、且只接受 EUC-JP/EUC-KR」这一条规则；GB18030 兜底、BOM/增量解码逻辑不变；不加依赖。
- 前端 13 项菜单顺序：原 9 项不动，新 4 项按 EUC-JP、EUC-KR、KOI8-R、KOI8-U 追加在末尾。
- 逐任务 TDD；四门绿再提交；commit 前 CHANGELOG；commit 消息无空格；commit 后 push；`--no-ff` 合 `main`。

## Review Focus

1. `as_str` / `from_label` / `encoding()` 三处映射一一对应，不出现串位。
2. 双 pass 的接受集只有 `EucJp | EucKr`——中文样本无 hint 仍是 GBK（探针已验证），不得翻转中文判定。
3. `encode_all` 的 `?` 替换对新编码同样不产生 `&#…;` 数字引用。
4. 前端断言与 `ENCODINGS` 表一致；`statusbar` 未知标签兜底路径不被 13 项影响。

---

## Task 1: Rust 编码标签 + 检测映射与双 pass

**Files:** Modify `crates/lumelog-core/src/encoding/mod.rs`、`tests/encoding_proptest.rs`；Test 同文件
**Interfaces:** `EncodingLabel::{EucJp,EucKr,Koi8R,Koi8U}`、`as_str/from_label/encoding/label_from_encoding` 扩展、`detect` 二次确认；`label_pool()` 加 4 项。
- [ ] **Step 1：测试**：`eucjp_euckr_koi8r_roundtrip`（各自编码→`decode_all` 还原、`invalid_count=0`）、`korean_emoji_is_unmappable`（EUC-KR 编 emoji → 单字节 `?`、无 `&#` 引用）、`from_label_accepts_new_labels_and_aliases`、`detect_euckr/eucjp/koi8u_samples`（约 1KB 真实语句编码后 `detect`）。
- [ ] **Step 2：** FAIL → 实现（`detect` 双 pass 见 spec §1.1）→ PASS；`cargo test --workspace` + `clippy -D warnings` 净。
- [ ] **Step 3：** CHANGELOG；commit `feat:encoding-euckr-eucjp-koi8` + push。

---

## Task 2: 前端菜单 + 断言

**Files:** Modify `app/frontend/src/encoding-menu.ts`、`app/frontend/src/ipc.test.ts`
**Interfaces:** `ENCODINGS` 13 项。
- [ ] **Step 1：** 更新 `ipc.test.ts` 断言为 13 项 → FAIL → 改 `ENCODINGS` → PASS。
- [ ] **Step 2：** `npm test` + `npm run build` 绿。
- [ ] **Step 3：** CHANGELOG；commit `feat:encoding-menu-13` + push。

---

## Task 3: 文档收尾

**Files:** Modify `docs/specs/2026-10-24-lumelog-v1-design.md`（§5.1）、`docs/features.md`（条目移已交付 + 笔误修正 + UTF-32 不做说明）、`docs/test-checklist.md`（§2 冒烟补样例行）、`CHANGELOG.md`
- [ ] **Step 1：** 三份文档更新。
- [ ] **Step 2：** CHANGELOG；commit `docs:encoding-extension-delivery` + push；`--no-ff` 合 `main` + push。
