# 插件接口 增量3：wasmi Wasm 解析插件 实现设计（Plan 23）

依据立项文档 `2026-10-30-plugin-wasm-proposal.md`：路线 **B（wasmi 2.0 解释器，+2–3MB）**，
接口采用**朴素 wasm ABI**（非 wit-bindgen——首版把 ABI 面压到最小，wit 冻结推迟到有第二实现时）。

## 1. 插件 ABI（`plugins/*.wasm`，wasm32-unknown-unknown）

模块必须导出：`memory`、`alloc(len: i32) -> ptr`、`meta() -> i64`、
`parse(input_ptr: i32, input_len: i32, base_line: i64) -> i64`、`finish() -> i64`。

- 打包约定：返回值 `(ptr << 32) | len`，指向插件 `memory` 内的 **UTF-8 JSON**；
- `meta()` → `{"id","name","extensions","columns"}`（与增量2 元数据同形）；
- `parse(chunk, base_line)` → `[{"line": u64, "cells": [string…]}]`；
  chunk 是**完整行**（含行尾 `\n`，仅 EOF 前最后一块可无），`base_line` 为块首行号；
  约定 `line = base_line + 块内换行计数`；
- 宿主流程：`alloc` → 写入块字节 → 置 fuel → `parse` → 读回 JSON → 发批；
  EOF 后调 `finish()` 冲刷插件内部缓冲的尾记录；
- 块大小 256KB；**fuel 每次调用重置 5×10⁸**（单次死循环被截断），另有总 deadline 与取消；
- 内存上限 `StoreLimitsBuilder.memory_size(16MB)`；wasm 文件 ≤8MB。

## 2. 集成

- `parser_plugins_list` 扩展：`*.js`（QuickJS）与 `*.wasm`（wasmi 实例化 + `meta()`）同表列出，
  文件名序、id 去重 first wins、单文件失败不阻塞、LIST_BUDGET 沿用；
- `parser_run` 按扩展名分派（`.js` → run_parser_plugin；`.wasm` → run_wasm_parser）；
  运行循环：`DocLines` 逐行累积至 256KB → `parse` → 512 行/批发帧（前端/表格/导出零改动）；
- 新 `wasmi_host.rs`：WasmPlugin（instantiate/meta/parse/finish + fuel/limits）与运行循环；
  `plugins.rs` 只做分派。

## 3. 插件作者面

文档给 Rust 模板（`crate-type = ["cdylib"]`、`#![no_std]` + panic handler、JSON 手拼或 serde
+ `serde-json-core`）；Go/TinyGo 需实现同一 ABI（文档列出导出清单即可）。

## 4. 不做

wit/wit-bindgen（ABI 冻结后再议）；流式宿主导入函数（`host_write` 等）——首版纯拉模型；
wasm 线程/SIMD；wasmtimer 之外的时间源；调试器支持。

## 5. 验收轮廓

- 测试 fixture 用 `wasm-encoder` 程序化构建 wasm 模块（无需 wasm 工具链）：
  常量 meta JSON + 常量行输出 + 空 parse 两类；
- 端到端：列表出 wasm 插件、`parser_run` 出行、finish 冲刷、取消、fuel 耗尽报错、
  缺导出/坏 wasm 报错不阻塞其它插件；
- 安装包体积门槛核对（P6 ≤15MB 挂账）；`cargo test --workspace` + `clippy` 净；
  `npm test`/`build` 回归（前端零改动，typeof 不变）。
