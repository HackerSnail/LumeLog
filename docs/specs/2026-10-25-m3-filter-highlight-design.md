# M3 子项目 A · 增量1：只读日志 行过滤 + 高亮命中 设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` §1（v1 M3 范围）、§6.4（行过滤仅只读 + 时序退化 + 过滤结果导出）、§4.2/§4.3（只读虚拟列表、≤200MB 编辑/只读分档）、§3（数据按块流动、绝不整文件驻留）、§6.1（正则方言差异只在搜索框旁提示，不做双引擎校对）。
>
> 本文只定义 **子项目 A 的第一增量**：只读视图上的**多条件 AND 行过滤** + **命中区背景高亮** + **命中间跳转** + **实时取消**。**时序筛选** 与 **`export_lines`** 属第二增量（依赖本期结果），另行设计。

**Goal:** 在只读大日志视图上，按多条"包含/排除 × 字面/正则"条件（AND）过滤出命中行、对命中区间加背景高亮并可在命中间跳转，全程流式、可取消、不把整文件文本驻留内存。

**Architecture:** 后端 `lumelog-app` 新增 `filter_run`（异步 `spawn_blocking` + 早返 req_id + 复用取消注册表，镜像已改造的 `can_parse`）逐行求 AND 谓词、流式回命中行号与行内命中区间（**cu16 码元坐标，后端用 core `CharMap` 换算好，`char_map` 本身不过 IPC——尊重现有契约**）；新增 `doc_lines_at` 按可见过滤窗口批量取行文本。前端 `listview.ts` 在既有 `visibleWindow` 虚拟化之上新增"子集视图"（渲染命中行数组），`main.ts` 把 `Ctrl+Shift+F` 单框搜索升级为统一"查询面板"（多条件行 + 两个总开关）。

**Tech Stack:** Rust（`lumelog-app` 命令层 + `lumelog-core` 匹配原语 + `tauri::ipc::Channel`）、TypeScript 原生 DOM（Vite + CodeMirror 不在本期）、`regex`、`memchr`。测试 `cargo test` / `vitest`，类型门 `tsc --noEmit`，静态门 `cargo clippy -D warnings`。

## 1. 范围与非目标

**做（本期）：**
- 只读模式（`>200MB` 虚拟列表）下的**视图级行过滤**：任意多条"包含" + 任意多条"排除"条件，AND 语义；每条可选 字面/正则 + 全局大小写开关。
- **命中区背景高亮**：对命中行内的匹配字节区间加背景色（Notepad++ Find-Style 式），保留原始行号。
- **命中间跳转**：F3 / Shift+F3 在命中行之间循环跳转，命中计数展示。
- **实时取消**：`filter_run` 扫描进行中可"停止"（`filter_cancel`），复用与 `can_parse` 一致的异步/取消/流式范式；顺带把既有 `search_run` 一并异步化接回 `search_cancel`。

**不做（本期后 / 明确非目标）：**
- 时序筛选、`export_lines`（第二增量）。
- 编辑模式（`≤200MB` CodeMirror）过滤：spec §6.4 定"仅只读隐藏行"。编辑模式高亮已由 M1 CodeMirror search 插件提供，不重做、不留第二套。
- 外部插件/脚本（主 spec §1 v1 非目标）。
- 后端多引擎正则方言校对（§6.1：字面/正则语义以后端 `regex`/`memchr` 为准，UI 旁提示）。

## 2. 语义定义（AND 谓词）

设包含条件集 I、排除条件集 X。对某行 `L`：
- `keep(L) = (∀ c∈I: match(L,c)) ∧ (∀ d∈X: ¬match(L,d))`。
- I 为空 ⇒ 包含侧恒真（等价"未按内容筛选"，仅 X 生效做剔除）。
- X 为空 ⇒ 无排除。
- I 与 X 皆空 ⇒ 不过滤（本期 UI 在无有效条件时不发起扫描，避免退化全表）。
- `match(L,c)`：字面用大小写敏感的子串包含（`caseSensitive=false` 时大小写无关）；正则用 `regex` crate（`caseSensitive=false` 走 `(?i)`）。与 `lumelog-core::search` 现有字面/正则求值语义保持一致，复用其原语，不新造方言。

**命中区间（高亮）**：仅对 I 中的各条件在其命中的跨度产出 `Cu16Range { cu16Start, cu16Len }`（同一条件多个匹配产多个）；排除条件 X 不产区间。坐标是**行内解码后文本的 UTF-16 码元偏移**——后端在扫描时用 core `CharMap::byte_to_line_cu` 把 `regex`/`memchr` 给出的字节跨度换算为行内 cu16（与既有 snippet 定位、`Match.byte_in_line` 同源），**`char_map` 本身不出 IPC**（遵 `dto.rs` 既定契约）。前端 `String.slice(cu16Start, cu16Start+cu16Len)` 直接切 span（JS 字符串索引即 UTF-16 码元），无需任何换算。

## 3. 后端接口（`lumelog-app` commands）

### 3.1 `filter_run`（新增，异步 + 流式 + 可取消）
- 入参：`{ doc_id: u64, conditions: Vec<FilterCondition>, case_sensitive: bool, on_batch: Channel<Vec<LineHit>>, on_done: Channel<FilterStats> }`。
- `FilterCondition { mode: "include"|"exclude", kind: "literal"|"regex", text: String }`（serde `rename_all="camelCase"`）。
- `LineHit { line: u64 /* 0-based 绝对行号 */, ranges: Vec<Cu16Range /* {cu16Start:u32, cu16Len:u32}；后端已用 core `CharMap` 把字节跨度换算为行内 UTF-16 码元坐标 */> }`。
- `FilterStats { kept: u64, scanned: u64 }`（`scanned`= 已判定行总数；`kept`= 命中行总数）。
- 形态：同步 prologue 校验 `doc_id`（未知 → `unknown_doc`）与每条 `text`（正则非法 → `bad_regex`/`regex_compile`，经 `IpcError::from_can`）；随即 `tauri::async_runtime::spawn_blocking` 后台扫描，命令**立即 `Ok(req_id)` 返回**。`req_id` 走既有 `begin_search/end_search/cancel` 通用取消注册表。
- 扫描：按 `doc.chunk_at` 分块解码取行，逐行求 §2 谓词；命中行连同区间累积到 `PARSE_BATCH`（沿用 512 量级）即 `on_batch.send`；每行判前查 `cancel` 标志早停。结束 `on_done.send(FilterStats)` + `end_search(req_id)`。
- **文本不驻留**：`filter_run` 只流式回命中行号 + 区间，**不回行文本**（守 §3）。
- **区间驻留上限（决策）**：命中行数无上限（行号以紧凑数组驻留前端可接受，百万级约数 MB）；但每行 `ranges` 全量驻留可能过大。设 `HIT_RANGE_CAP`（默认 50_000）：前 `HIT_RANGE_CAP` 条命中行带回区间；超出部分 `ranges` 置空（前端对这些行退化为"整行淡色"无区间高亮）并 `on_done` 附 `truncated:true`（`FilterStats` 加 `truncated` 布尔）。理由：典型过滤命中是小众；病态"几乎全命中"时降级区间而非 OOM。

### 3.2 `doc_lines_at`（新增，按需取子集文本）
- 入参：`{ doc_id: u64, indices: Vec<u32> /* 0-based，升序，来自可见过滤窗口 */, on_data: Channel<LineContentPayload> }`。
- `LineContentPayload { lines: Vec<IndexedText /* {index:u32, text:String} */> }`——**只回行文本**，不回 `char_map`（高亮区间已由 `filter_run` 以 cu16 给出）。命名/形态与既有 `ChunkPayload` 对齐，逐条 `{index,text}`。
- 语义：对给定（可能稀疏的）原始行号集合，取各行**解码后文本**。单次请求行数上限沿用 `MAX_CHUNK_LINES`；超限 `invalid_argument`。后端可分组连续段取块减少 I/O。
- 仅在"过滤到命中行"开启时的可见窗口调用；关闭过滤时列表仍走既有连续 `doc_chunk`/`loadWindow` 路径，不改。

### 3.3 `search_run` 异步化（改造既有）
- 由同步改为与 `filter_run` 同构：`spawn_blocking` + 立即返回 `req_id` + 每批 `on_batch` + `on_done`；接回既有 `search_cancel(request_id)` 真实取消（此前因同步而无实时取消按钮，本次补上）。保持既有单模式搜索语义（命中列表 + snippet + 跳转）。

## 4. 前端

### 4.1 查询面板（`main.ts`，替换旧单框搜索 UI）
- 条件行：`[包含/排除 ▾] [字面/正则 ▾] [文本输入] [× 删除]` + "＋ 添加条件"。全局：`大小写敏感` 勾选、`过滤到命中行` 开关、`高亮命中` 开关、`上一个/下一个`（Shift+F3/F3）、命中计数、`停止`（进行中）。
- 行为：点"应用/回车"→ 若有开关关闭（未过滤仅高亮）也需扫描得命中集合；`re-entrancy` 守卫同 `parseActive`（一次仅一个扫描）；进行中按钮转"停止"接 `filterCancel(req_id)`。
- 快捷键：`Ctrl+Shift+F` 打开面板；`F3` 下一命中、`Shift+F3` 上一命中（作用于当前活动只读文档）。

### 4.2 子集视图（`listview.ts`）
- 新增可选模式：持 `hits: {line:number, ranges:Cu16Range[]}[]`（升序、按 `line`）。开启过滤时 `total()` 返回 `hits.length`，`visibleWindow(scrollTop, h, hits.length)` 得 `[first,last)`；可见窗口的原始行号 = `hits[first..last].line` → `doc_lines_at` 批量取行文本 → 渲染行（行号列显示原始 `line+1`），行内区间用 `String.slice` 按 cu16 直接切 `<span class="line-hl">`（JS 字符串索引即 UTF-16 码元，无需 `char_map`）。
- 关闭过滤时维持既有连续虚拟化路径不变（`loadWindow`），仅叠加高亮（命中行的区间来自 `hits`）。
- `scrollToLine`/`revealLine` 语义在过滤模式改为按 `hits` 下标定位（跳转命中共用）。
- 取文本并发保护沿用既有 `seq` 竞态守卫 + 缓存窗口。

### 4.3 IPC/DTO（`ipc.ts` / `dto.rs`）
- 新增 `filterRun({docId, conditions, caseSensitive, onBatch, onDone}): Promise<number>`、`filterCancel(requestId)`、`docLinesAt({docId, indices, onData})`（镜像 `canParse` 的双 Channel + 早返 id 包装）。
- `FilterCondition`/`LineHit`/`ByteRange`/`FilterStats` 类型前端对齐 `dto.rs`（camelCase + `skip_serializing_if` Option）。

## 5. 错误与边界
- 空条件：不发起扫描（UI 提示"请至少添加一条条件"）。
- 非法正则：同步 prologue 拒绝（`bad_regex`/`regex_compile`），不流式空结果。
- 未知 doc / 互斥锁中毒：沿用既有 `unknown_doc` / `io` 码。
- 取消：早停后 `on_done` 仍发（含已命中计数 + `truncated`）；`end_search` 清注册表（每路径恰好一次）。
- 编码：行文本按文档当前编码解码（`doc.encoding()`）；不可映射字节计数继续走既有 ⚠ 通道，不影响命中判定（在解码后文本上匹配）。
- 后端字节→cu16 换算越界/坏字节：该区间不产出（前端无对应 span），不 panic、不影响行渲染。

## 6. 全局约束（沿用）
- UI 文案简体中文；DTO `#[serde(rename_all="camelCase")]` + `Option` 用 `skip_serializing_if`。
- 不整文件驻留（§3）：`filter_run` 回行号+区间、文本按需 `doc_lines_at`。
- 命令参数 camelCase↔snake_case 由 Tauri 自动转换。
- 逐任务 TDD（red→green）；每任务 `cargo test`/`cargo clippy --workspace --all-targets -- -D warnings`/`npm test`/`run build` 全绿再提交；commit 前追加根 `CHANGELOG.md`（时间倒序、写清文件/原因/验证）；commit 消息无空格无特殊字符（Windows shell）；commit 后直接 push；merge 走分支 `--no-ff`。

## 7. 验收

**可测（headless 门）：**
- `lumelog-core`：AND 谓词求值器纯函数——包含全真、任一排除即剔、字面大小写、正则 `(?i)`、I 空恒真、X 空恒真、皆空不扫；产区间与多匹配。
- `lumelog-app`：`filter_run` 在夹具日志上断言命中行集合 + 顺序 + 区间 + 取消早停 + `truncated` 降级 + camelCase DTO 序列化；`doc_lines_at` 对稀疏升序索引取回正确行文本（不含 `char_map`）；`search_run` 异步后取消真实生效。
- 前端纯逻辑：`hits` 子集 → 可见窗口原始行号映射、`hits.length` 驱动的 `visibleWindow`、cu16 区间用 `String.slice` 切 span 的正确性（多字节/astral 行；字节→cu16 换算在后端由 `CharMap` 完成、与既有 `byte_to_line_cu` 用例对齐）。

**GUI 手工（`tauri dev`，headless 以单测 + tsc 为门）：**
1. 打开一份 >200MB 只读日志；查询面板加 1 条"包含 正则"，勾"过滤到命中行"：列表只显命中行、行号为原始值、可滚动。
2. 叠加"排除 字面"：结果同时满足 AND；计数正确。
3. 开"高亮命中"：命中区间有背景色；F3/Shift+F3 循环跳转。
4. 扫描进行中点"停止"：及时中止、保留已得结果、按钮复位。
5. 极端"几乎全命中"：触发 `truncated` 时不 OOM、退化为整行淡色 + 提示。

**性能对齐主 spec：** P4（滚动/翻页）在过滤模式仍只取可见窗口行；P3 内存——行号数组驻留、文本不驻留；扫描与既有索引同为流式可取消。

## 8. 分期（本 spec → 计划切分预告）
- 计划任务大致：① core AND 谓词纯函数 + 单测 → ② `filter_run` 命令（异步/取消/流式/DTO）+ 集成测 → ③ `doc_lines_at` + 测 → ④ `search_run` 异步化改造 → ⑤ 前端 `ipc.ts` 包装 + 类型 → ⑥ `listview.ts` 子集视图 + 映射/换算纯函数 + 测 → ⑦ `main.ts` 查询面板 + F3 + 停止 + 联动 → ⑧ 样式（`line-hl`/面板）+ 收尾验收。
