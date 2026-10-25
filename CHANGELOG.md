# Changelog

按 git log 时间倒序（最新在最上方）。每条写清：改了哪些文件、为什么、验证结果。

## 2026-10-25 — Plan 4 M3 行过滤 + 高亮命中（feat/m3-filter-highlight）

**依据：** spec `docs/specs/2026-10-25-m3-filter-highlight-design.md`；计划 `docs/plans/2026-10-25-m3-filter-highlight.md`（M3 子项目 A 增量1：只读视图 多条件 AND 行过滤 + 命中区高亮 + F3 跳转 + 实时取消）。

### 终审修复波（CodeReview 全分支 0 Critical / 5 Important / 6 Minor）
**Files:** Modify `app/frontend/src/listview.ts`、`app/frontend/src/main.ts`。
**Why（逐条）：**①F3 被 window 层无条件劫持、与编辑器 CodeMirror searchKeymap 双响应 → 改为仅当活动文档是只读列表（`lists.has`）才接管，否则放行编辑器；②`handleClose` 漏清 `queryHits/queryCursor` 且迟到的 `onDone` 会重填已闭文档（docId 单调不复用→永久泄漏）→ handleClose 补删两图 + onDone 首行 `docStates.has` 守卫；③子集模式下 `scrollToLine/revealLine` 未按 hits 重映射、CanPanel/Ctrl+G 跳转错位且 `paintSubset` 不读 `flashIndex` → 新增 `slotForLine` 二分映射 + 子集 `paintSubset` 闪烁判定；④换编码后沿用旧解码算出的 cu16 区间与旧命中行集（同“行文本一致”约束的编码维度）→ `changeEncoding` 列表分支 `setView(null)`+清 hits+提示；⑤扫描中重开面板丢弃“停止”入口并把流式计数写进已 detach 旧节点 → `openQueryPanel` 在 `queryBusy` 时复用现有 DOM不重建。另修 Minor：⑦子集取文回传 `invalidCount` 使 §5.4 ⚠ 不再冻结（loadLines 改为返回全 payload）；⑨`await filterRun` 后补 `if(!queryBusy) queryReqId=null` 防早完成写完已结束 id；⑳声明 `queryBusy` 为全局单扫描互斥的有意行为。**Ruling（park）：**⑥“整行淡色”截断退化本轮只留计数提示（区分“截断空区间”与“仅排除空区间”需额外标记，价值低）→ M3 第2增量；⑪命中行以 JS 对象数组驻留（百万级约 100MB而非 spec 估的“数 MB”），典型命中稀少可接受，建议二增量改 `Int32Array`+稀疏区间 Map。Minor⑧（`setView(null)` 无调用方）由修复④自然获得调用。
**验证:** `npm --prefix app/frontend test` 46 pass + `run build`（tsc 通过）；Rust 本轮无改动（上轮 workspace 测试+clippy 已净）。待定向复审（fix range）后 merge main。

### Task 1. lumelog-core 行过滤求值器（纯函数）
**Files:** Create `crates/lumelog-core/src/filter.rs`；Modify `crates/lumelog-core/src/lib.rs`（`pub mod filter;`）。
**Why:** 提供 spec §2 的 AND-of-includes / NOT-of-excludes 谓词，产**行内 cu16 高亮区间**（`char_map` 不过 IPC），与渲染文本逐字符一致。`LineFilter::build` 编译各条件（字面走 `regex::escape`、大小写走 `RegexBuilder::case_insensitive`），`keep(line_text)->Option<Vec<Cu16Range>>`（Some=保留+包含命中的 cu16 区间升序去重；None=剔除；仅排除时 Some(空)）。字节→cu16 用前缀 `chars().map(len_utf16).sum()`。空条件文本/非法正则 → `CoreError::InvalidPattern`。
**验证:** `cargo test -p lumelog-core filter::` 10 pass（AND 语义/排除否决/仅排除恒真/大小写/字面元字符/多字节 cu16 偏移 `错误 <emoji>`→start3 len2/空模式拒/非法正则拒/has_conditions 空假/含全真）；`cargo clippy -p lumelog-core --all-targets -- -D warnings` 净；core 其余 58 测不回归；前端无改动。

### Task 2. 过滤 DTO（`lumelog-app`）
**Files:** Modify `app/src-tauri/src/dto.rs`（新增 `FilterCondition`(+`to_core`)、`Cu16RangeJson`(+`From<Cu16Range>`)、`LineHit`、`FilterStats{kept,scanned,truncated}`、`IndexedText`、`LineContentPayload`，均 camelCase；+`filter_dto_tests`）。未接入命令前对 6 结构体/`to_core`/`From` impl 加带注释 `#[allow(dead_code)]`（Task 3/4 接入后剥除）。
**Why:** 为 `filter_run`/`doc_lines_at` 定IPC 契约。**Ruling：**过滤模式编译错误（空条件/非法正则）由 `LineFilter::build` 返 `CoreError::InvalidPattern`，`IpcError::from_core` 已映射为 `"invalid_pattern"`（ipc.ts 已有中文表）——不再另设 `bad_regex`（那个码专属于 CAN 正则源），与 M1 搜索一致、零分叉。
**验证:** `cargo test -p lumelog-app` 16 pass（新 4 个 `filter_dto_tests`）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 3. `filter_run` 异步流式过滤 + `filter_cancel`
**Files:** Modify `app/src-tauri/src/commands.rs`（新增纯函数 `scan_filter<I: Iterator<Item=Line>>` + `#[tauri::command] filter_run`（异步 prologue 校验/编译条件 → `spawn_blocking` 后台扫全文件、立即 `Ok(req_id)`）+ `filter_cancel`；新常量 `HIT_RANGE_CAP=50_000`；+`filter_run_tests`）；Modify `app/src-tauri/src/lib.rs`（注册两命令）；Modify `dto.rs`（剥除 Task3 六处 `#[allow(dead_code)]`，保留 IndexedText/LineContentPayload 到 Task4）。
**Why:** spec §3.1 只回命中行号+cu16 区间（文本不过 IPC，守 §3），`scan_filter` 走 `DocLines` 惰性逐行、每行判取消、`PARSE_BATCH` 聚合；区间驻留超 `HIT_RANGE_CAP` 降级空区间 + `truncated`。异步化同 `can_parse`：取消可用、不冻结主线程、流式实时渲染。**Ruling：**`scan_filter` 设设泛型迭代器（而非计划的 `&mut DocFile`）以便无 `DocFile` 进单测；空条件不扇出全文件扫描（prologue 直发 `on_done{0,0,false}`）。
**验证:** `cargo test -p lumelog-app` 20 pass（新 4 个 `scan_filter_*`：AND 保留行集/升序、cu16 区间、取消早停、超 cap 截断）+ `cargo clippy --workspace --all-targets -- -D warnings` 净；前端无改动。

### Task 4. `doc_lines_at` 按需取子集行文本
**Files:** Modify `app/src-tauri/src/commands.rs`（新增纯函数 `gather_lines(&mut DocFile, &[u32])` + `#[tauri::command] doc_lines_at` + `doc_lines_at_tests`）；Modify `app/src-tauri/src/lib.rs`（注册）；Modify `dto.rs`（剥除 IndexedText/LineContentPayload 最后两处 `#[allow(dead_code)]`，已全部消除）。
**Why:** 过滤子集视图只渲染命中行，需按稀疏原始行号批量取文本（守 §3，不驻留全文）。`gather_lines` 逐行 `chunk_at(i,1)` 并按 `loadWindow` 同法去尾换行（`\n` 然 `\r`），与 `filter_run` 的 cu16 区间逐字符对齐；`indices.len()>MAX_CHUNK_LINES` → `invalid_argument`。
**验证:** `cargo test -p lumelog-app` 23 pass（新 3 个 `gather_lines_*`：子集取回、CRLF 去尾、超 cap 拒）+ `cargo clippy --workspace --all-targets -- -D warnings` 净；前端无改动。

### Task 5. 前端 IPC 桥接（`ipc.ts`）
**Files:** Modify `app/frontend/src/ipc.ts`（新增 5 个类型 `FilterCondition`/`Cu16Range`/`LineHit`/`FilterStats`/`IndexedText` + 3 个包装 `filterRun`（双 Channel、解为 req_id）/`filterCancel`/`docLinesAt`，均 camelCase 对齐 dto.rs）；Modify `app/frontend/src/ipc.test.ts`（+2 例：包装为函数、三复用错误码有中文）。
**Why:** spec §4.3—镜像 `canParse` 双 Channel 形状；`invalid_pattern`/`unknown_request`/`invalid_argument` 已在既有联合与中文表，无需新增错误码。
**验证:** `npm --prefix app/frontend test` 41 pass（ipc 7→9）+ `run build`（tsc 通过，41 modules）。

### Task 6. `listview.ts` 子集视图 + 高亮切分
**Files:** Modify `app/frontend/src/listview.ts`（新增导出纯函数 `sliceLineTextByRanges(text, Cu16Range[])->TextSeg[]`；ListView 新增子集视图态 `hits/hideNon/hitMap` + `setView(hits,hideNon,highlight)` + `scrollToHit(i)` + `viewTotal()`；render 分支子集/连续，子集走 `opts.loadLines`(doc_lines_at) 按可见窗口稀疏取文本、缓存按原行号键共享；paint/paintSubset 用 `renderText` 叠加 `.line-hl` span）；Modify `app/frontend/src/listview.test.ts`（+5 `sliceLineTextByRanges_*`）。
**Why:** spec §4.2。cu16 区间用 JS `String.slice` 直切（索引即 UTF-16 码元），重叠/相邻区间先合并去重、越界丢弃（不重复 emit、不在代理对中间切，后端区间按字符对齐）。模式切换重置 renderFirst/Last 强制新窗口（Review Focus 4）；连续快路径不变（仅命中行走切分）。
**验证:** `npm --prefix app/frontend test` 46 pass（listview 5→10）+ `run build`（tsc 通过）；Rust 无改动。

### Task 7. `main.ts` 查询面板（取代旧单框搜索）
**Files:** Modify `app/frontend/src/main.ts`（删 `openSearchFlow/renderSearchShell/runSearch/appendHits` 与 `searchRun`/`MatchJson` 导入；新增查询面板 `openQueryPanel`（多条件行 `addCondRow`/`collectConditions` + “过滤到命中行”/“高亮命中”/“区分大小写” + 应用/上一个/下一个/停止/关闭 + 状态）、`applyQuery`（`filterRun` 流式收集 hits→`onDone` 单点 `list.setView(hits,hide,hl)`、重入守卫、`queryReqId` 接 `filterCancel`）、`cycleHit`、`fetchLines`（`docLinesAt` Promise 包装）；`mountList` 注入 `loadLines`；appbar/命令面板/快捷键 `Ctrl+Shift+F` 改指 `openQueryPanel`；`registerShortcuts` 新增 F3/Shift+F3（无 mod））。
**Why:** spec §4.1。面板统一走 `filter_run`（单条包含=原单模式搜索+高亮），**Ruling：旧 `search_run` 保留注册但退出前端主路径**（不重复异步化，避免两套正则链路）；`queryBusy` 仅在 `onDone`/catch 置否（filterRun 提前解 id）；hits 每 doc 缓存供 F3 循环（`queryHits`/`queryCursor`）。视图切换保留各 doc 的 list 实例，子集态自然持久。
**验证:** `npm --prefix app/frontend run build`（tsc 通过，41 modules，openQueryPanel 已接）+ `npm test` 46 pass；grep 确认 `openSearchFlow/runSearch/renderSearchShell/appendHits/searchBusy` 已清除、`jumpToLine` 仍被 Ctrl+G 复用；无新单测（DOM/IPC 密集，GUI 手工验收留 Task 8）。

### Task 8. 样式 + 全量验收
**Files:** Modify `app/frontend/src/styles.css`（查询面板 `.query-conds`/`.query-cond`/`.query-chk` 与高亮 `.line-hl`（半透明琥珀色，三主题通用无需 per-theme token）；旧 `.search-head input` flex 仅作用文本框，新增 `.query-chk input{flex:0 0 auto}` 防勾选框拉伸）。
**Why:** spec §7。面板复用 `#search-panel`（未改 index.html）；条件行走 `.query-conds` 独立滚动列，行多不顶走动作栏。
**手工验收清单（需 `tauri dev`，headless 以单测+tsc 为门）：**① 打开 >200MB 只读日志→Ctrl+Shift+F 查询面板→加 1 条包含正则→勾“过滤到命中行”→只显命中行、行号原值；② 叠加排除字面→AND 生效、计数对；③ “高亮命中”开→命中区间背景色 + F3/Shift+F3 循环；④ 扫描中点“停止”→及时中止保留已得、按钮复位；⑤ 切标签回原 doc→子集视图保留。
**验证（全量四门）:** `cargo test --workspace` 全绿（lumelog-app 23含 scan_filter 4+gather_lines 3、lumelog-core 68含 filter 10、lumelog-can 集成套件、cli_smoke 9/1 ignored、docfile/proptest）+ `cargo clippy --workspace --all-targets -- -D warnings` 净；`npm test` 46 pass + `run build`（tsc 通过）。

## 2026-10-25 — 收尾核查：修复 `main` 上 canpanel.ts 回归（build 断）

**背景：** Plan 3 合入 main 后的全量回归核查。发现 `main`（merge `7d8c79b`）的 `app/frontend/src/canpanel.ts` 中 `onScroll` 调用 `this.paintNow()`，但 `paintNow()` 方法定义未写入——修复波后续提交 `90278a5` 当时只持久化了 onScroll 那一行、`render()`+`paintNow` 定义那半因编辑器保存间歇失败未落盘（当时“build 42 modules”为陈旧内存视图，非提交 blob），导致 `tsc` 报 TS2551（vitest 不跑 DOM 路径故未拦）。**验证发现：**`npm run build` 在 main 上失败。
**Files:** Modify `app/frontend/src/canpanel.ts`（补全 `paintNow()` 定义 + `render()` 的 rAF 改调 `paintNow()`，与已在的 `onScroll` 调用合并为单帧重绘）。走分支 `fix/canpanel-paintnow` → merge main（不直提 main）。
**验证：** `cargo test --workspace` 全绿（lumelog-app 12 + lumelog-can 全部 golden/candump/asc/regex/cli/proptest 套件 + lumelog-core 58 + chunk/cli/docfile/encoding 集成，1 ignored=手动 1GiB）；`cargo clippy --workspace --all-targets -- -D warnings` 净；`npm --prefix app/frontend test` 39 pass + `run build`（tsc 通过、41 modules）。教训：保存间歇失败时必须以 `git diff`/`tsc` 为准复核，不信任工具视图。

## 2026-10-25 — Plan 3 M2 CanPanel + IPC（feat/m2-can-panel）

**依据：** spec `docs/specs/2026-10-24-lumelog-v1-design.md` §7.4/§9；计划 `docs/plans/2026-10-25-m2-can-panel-ui.md`（把 `lumelog-can` 引擎接入 Tauri 壳 + CanPanel）。用户批复：用 TanStack、纳入 `can_decode_snippet`、过滤范围 ID/消息名/信号条件/时间窗口。

### 终审修复波（CodeReview 全分支 → Request changes，0 Critical / 6 Important / 7 Minor）
**Files:**
- Modify: `app/src-tauri/src/commands.rs`（① `can_parse` 同步 prologue 只做校验、随即 `tauri::async_runtime::spawn_blocking` 后台解码并**立即返回 req_id**——解码移出主线程、UI 不再冻结、流式批次实时可见；② `doc_lines`（整文件 `Vec`）重写为惰性 `DocLines` 迭代器，一次只持有一个 `MAX_CHUNK_LINES` 窗口（`chunk_to_lines` 抽出共享），符 §3“never hand the whole file over”；`AppHandle`+`Manager` 供 worker 结束 `end_search`；regex 用一次性 `RegexSource::new(empty)` 预校验；`#[allow(too_many_arguments)]` IPC 签名）
- Modify: `app/src-tauri/src/commands.rs`（`can_parse_tests` 三例改喂 `DocLines::new(Arc::new(Mutex::new(doc))).collect()`）
- Modify: `app/frontend/src/main.ts`（① 恢复“停止”：`parseActive` 拿到提前返回的 req_id 存入 `canReqId`，`onCanRunClick` 在 `canParsing` 时调 `canCancel`，`canParsing` 仅在 `onDone`（含取消）置否；④ `decodeSelection` 传 `sourceKind`（ASC 选中不再误当 candump）；⑤ 重入守卫移进 `parseActive` 开头（palette `can-parse` 无法并发灌帧）；Minor：`store.clear` 前先校验源/正则、`switchCanView` 复用已注册 store、清除/切换过滤时复位 `signalOp` 下拉）
- Modify: `app/frontend/src/canpanel.ts`（③ 表头 `hrow` 去 `can-row`（`position:absolute` 会抽流致 `.can-thead` 塌陷盖首行）单列 `renderHeader`；⑥ `FrameStore.revision` + 缓存 `cachedRows/cachedTable` 按 (store,rev,filter) 失效，`render()` rAF 合并 `onBatch` 突发，滚动路径 first/last 未变短路——去 O(n²) 重建）
- Modify: `app/frontend/src/canpanel.test.ts`（加 `frame_store_revision` 例）
- Modify: `app/frontend/src/styles.css`（③ `.can-hrow` 补 `height:20px`+`white-space:nowrap`，独立成流并与 `.can-row` 列对齐）

**Why:** 终审 6 项 Important 阻塞 merge，逐条修复。①②同源——同步命令既冻结主线程又让 req_id 只在结束时可得（取消形同虚设），一并异步化后取消才真实可用（推翻 Task 7 的“不提供停止”Ruling）。**Ruling（Minor 分诊）**：扩展帧 id 两层语义（依 Task 2 DBC bit31 还原，展示层不重复解码，改动会牵动 DTO 与已测夹具）、远程帧带数据解全 0 信号（can 语义边界、v1 展“无数据”已不崩）、DBC 只增不减（用户显式导入、数量极少）、非法过滤值静默忽略（`parseNum` 返 undefined 视作无上/下界，YAGNI）——四项均判为可在 M3 处理、非阻塞，按 final-review 规则记此 park 而非塞进本次修复波。**验证：** `cargo test -p lumelog-app` 12 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净；`npm --prefix app/frontend test` 39 pass（canpanel 2→3）+ `run build`（tsc 通过，41 modules）。

**定向复审（CodeReview 子代理，fix range a5d27a6..6f055cd）：** 6 项 Important 全部 ADDRESSED，无新增 Critical/Important。唯一新 Warning：⑥ 把 `render()` 改为 rAF 合并后，`onScroll` 自身已在 rAF 内又调 `render()` → 双层 rAF、纯滚动多一帧延迟。**后续修正**：抽出 `paintNow()`，`render()` 与 `onScroll()` 的 rAF 均直调它（滚动不再回绕 `render()`），保持单帧重绘；`npm test` 39 pass + `run build`（42 modules）重跑净。结论：可 merge。


### Task 7. main.ts 集成 CanPanel（报文 dock + 双向联动）
**Files:**
- Modify: `app/frontend/src/main.ts`（导入 CAN 命令 + `CanPanel/FrameStore` + `CanFilter`；报文 dock（源选择 candump/asc/正则 + 导入DBC + 解析/识别选中）+ 过滤栏（ID/消息/信号条件/时间窗）；每 docId 一份 `FrameStore`/`CanFilter`，共享一个 `CanPanel` 按活动文档重挂；行点击→`revealSourceLine`（列表 0-based、CodeMirror 1-based）；选中文本→`canDecodeSnippet`；appbar “报文面板”按钮 + 4 条 palette 命令；`handleClose` 清理 CAN 映射）
- Modify: `app/frontend/src/styles.css`（`#can-dock`/`.canpanel`/`.can-*` 确定高度 flex 链 + `.list-flash`）

**Why:** spec §7.4 CanPanel 接入主界面。行号对齐是 M2 验收核心：`FrameJson.lineNumber`（1-based）→ 列表 `revealLine(lineNumber-1)`（0-based）/编辑器 `revealLine(lineNumber)`。**Ruling（取消）**：`can_parse` 与 `search_run` 同为同步命令，只在整个扫描完成后才返回 req_id、且循环期间持有文档锁，因此前端无法在流式中途拿到 id 去 `can_cancel`——与仓内已有的 search-cancel 推诿（“需异步搜索”）同理，本次不提供会空转的“停止”按钮，改为解析中显示“解析中…”并提示；`can_cancel` 命令/包装保留为真实 API（已测），待 `can_parse` 异步化后接入。
**验证:** `npm --prefix app/frontend test` 38 pass + `npm --prefix app/frontend run build`（tsc 通过，41 modules，canpanel+table-core 入 bundle）；`cargo test -p lumelog-app` 12 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净。
**手工验收（需 `tauri dev`，headless CI 无法跑 GUI）：**① 打开 `crates/lumelog-can/tests/data/candump_basic.log` → 报文面板 → 导入 `sample.dbc`（EngineData/ExtFrame/MuxedMsg，1 处坏 SG_ 告警仍宽容导入）→ 源 candump → 解析：表格出帧；② 点行→跳回源码行（闪烁）；③ ID 区间过滤生效；④ 展开行显 Speed 信号 physical/raw；⑤ 错误/远程帧行不崩（dataHex 空显 –）；⑥ 选中一行→“识别选中”弹层显解码帧。⑥需编辑器环境。无 GUI 时以单测+tsc 作为门。

### Task 6. CanPanel 组件（table-core 列模型 + 行窗口虚拟滚动 + revealLine 联动）
**Files:**
- Create: `app/frontend/src/canpanel.ts`（`FrameStore` 累积批量/`all`/`clear`/`length` + `CanPanel` 手写 DOM 虚拟表：行号/时间/通道/ID/消息/DLC/信号/原始数据 8 列，表头取 `getFlatHeaders()`，体只画 `visibleWindow` 窗口 + 扩展子行，行点击→`onRowActivate(1-based lineNumber)`，ID hex/dec + 时间绝对/相对 + ms/µs 切换）
- Create: `app/frontend/src/canpanel.test.ts`（`FrameStore` 2 例，DOM-free）
- Modify: `app/frontend/src/listview.ts`（`revealLine(index0)`：`scrollToLine` + 一次闪烁高亮，`paint` 内施加、定时器清除、`destroy` 清理）
- Modify: `app/frontend/src/editor.ts`（`EditorHandle.revealLine(line1)`：`doc.line` 钉位 + `dispatch` selection + `EditorView.scrollIntoView`）
- Modify: `app/frontend/package.json` + `package-lock.json`（`@tanstack/table-core ^8`）

**Why:** spec §7.4 报文表格。用户定使用 TanStack—但 table-core 无框架适配器时 `createTable` 需受控 `state`，故首次捕获特性默认 `initialState`、后续回传作 `state`（空 state 会致特性首次 `getState()` 崩）；无 `flexRender`，表头文本直接取 `columnDef.header`（字符串）。行窗口沿用 `listview.visibleWindow`。闪烁高亮放在 `paint` 内因目标行可能还在异步 fetch。`canpanel.css`/`.list-flash` 样式随 Task 7 挂载一并补（未挂载时不影响逻辑）。
**验证:** `npm --prefix app/frontend test` 38 pass（新 `canpanel.test.ts` 2 例）+ `npm --prefix app/frontend run build`（tsc 通过，38 modules；canpanel 待 Task 7 接入才入 bundle）。

### Task 5. CanPanel 视图过滤引擎（纯函数）
**Files:**
- Create: `app/frontend/src/canfilter.ts`（`CanFilter` 接口 + `applyCanFilter(frames, f)` 纯谓词）
- Create: `app/frontend/src/canfilter.test.ts`（7 例）

**Why:** spec §7.4 / 用户批复的过滤范围（ID 区间/消息名/信号条件/时间窗口）。纯函数不改入参数组与帧对象（视图级过滤不动行索引）；ID 闭区间且 `idFrom>idTo` 直接不匹配（非法区间不抛错）；时间窗口下 `timestampMs` 缺失的帧被排除；信号 `==` 用 `Object.is` 钉死浮点语义。各轴 AND。
**验证:** `npm --prefix app/frontend test` 36 pass（新 `canfilter.test.ts` 7 例：no_filter/id 区间含单边界与反转/msg_name/signal 条件/time_window/组合 AND/不变更）+ `npm --prefix app/frontend run build`（tsc 通过，39 modules）。

### Task 4. 前端 `ipc.ts` CAN 桥接 + 新错误码
**Files:**
- Modify: `app/frontend/src/ipc.ts`（`SignalValueJson`/`FrameJson`/`DbcMsgSummary`/`DbcErrorJson`/`DbcInfo`/`CanStatsJson` interface（camelCase 对齐 `dto.rs`）+ `IpcErrorCode` 联合加 `dbc_syntax`/`bad_regex`/`regex_compile`/`source_format` + `IPC_ERROR_ZH` 四条中文 + `dbcImport`/`canParse`（双 Channel 抄 `searchRun`）/`canCancel`/`canDecodeSnippet` 包装）
- Modify: `app/frontend/src/ipc.test.ts`（`describe("CAN ipc bridge")` 两例：错误码含中文且 detail 代入、四包装均为函数）
- Modify: `Cargo.lock`（补记 Task 1/3 已提交的 `lumelog-can` 依赖 + `tempfile` dev-dep 的锁）

**Why:** spec §9 要求前端与 Rust DTO 字段逐一对齐、错误码→稳定中文文案；`canParse` 沿用 `search_run` 的批量流式双通道形态，`dbcId`/`regex` 缺省以 `null` 传递匹配 Rust `Option` 参数。
**验证:** `npm --prefix app/frontend test` 29 pass（ipc.test.ts 由 5→7，新 CAN 两例由红转绿）+ `npm --prefix app/frontend run build`（`tsc --noEmit` 通过，38 modules transformed）。

### Task 3. `can_parse` 流式命令 + doc_lines + can_cancel + can_decode_snippet
**Files:**
- Modify: `app/src-tauri/Cargo.toml`（dev-dep `tempfile = "3"`）
- Modify: `app/src-tauri/src/commands.rs`（`doc_lines`（`DocFile` 分块→`Vec<Line>` 1-based）、`parse_frames`（`&mut dyn FrameSource` 批量+逐帧取消）、`can_parse`/`can_cancel`/`can_decode_snippet`）+ 4 单测
- Modify: `app/src-tauri/src/lib.rs`（注册三命令）

**Why:** spec §9 `can_parse` 采 `search_run` 的 Channel/批量/取消骨架（`PARSE_BATCH=512` 防 IPC 风暴、逐帧查共享 cancel 标志）；行数按 `stats()->SourceStats`。`can_decode_snippet` 支撑§7.4 “识别为报文”（选中行走单行源）。接入后剥除 Task 1/2 的 stale `#[allow(dead_code)]`。
**验证:** `cargo test -p lumelog-app` 12 pass（新：`doc_lines_preserves_line_order`（CRLF 保序/去行尾）、`parse_frames_decodes_candump_and_counts_skips`（2 解/1 跳/1.5s→1500ms/1-based 行号）、`parse_frames_stops_on_cancel`（早停不 panic）、`frame_json_reconstructs_data_hex_for_full_payload`（dlc=64→128 hex 不越界））+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净 + `cargo check --workspace` 通过。

### Task 2. DbcStore + `dbc_import` 命令
**Files:**
- Modify: `app/src-tauri/src/state.rs`（`AppDocs` 加 `dbc: Mutex<HashMap<u64, Arc<Database>>>` + `next_dbc_id` + `insert_dbc`/`get_dbc` + `dbc_store_roundtrip` 单测）
- Modify: `app/src-tauri/src/commands.rs`（`import_dbc_info`（读文件 + `from_utf8_lossy` + `parse_lenient`）、`dbc_summaries`（报文行扩展 id 按 bit31 还原）、`#[tauri::command] dbc_import`）+ 2 单测
- Modify: `app/src-tauri/src/lib.rs`（注册 `dbc_import`）

**Why:** spec §9 `dbc_import` → `{dbc_id, messages, errors}`。走宽容解析使单行坏构造不致整库导入失败（§7.1）；缓存 `Database` 供 `can_parse` 按 `dbc_id` 取用。
**验证:** `cargo test -p lumelog-app` 8 pass（新：`dbc_store_roundtrip`、`dbc_import_reads_sample_dbc`（EngineData 保留 2 信号/1 个 `bad SG_` 告警/ExtFrame id=2147484928/MuxedMsg 3 信号）、`dbc_import_missing_file_is_io_error`）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 1. 接入 lumelog-can + CAN DTO + from_can
**Files:**
- Modify: `app/src-tauri/Cargo.toml`（加 `lumelog-can` 路径依赖）
- Modify: `app/src-tauri/src/dto.rs`（`SignalValueJson`/`FrameJson`/`DbcMsgSummary`/`DbcErrorJson`/`DbcInfo`/`CanStatsJson` camelCase serde + `frame_json`/`signal_value_json` + `IpcError::from_can` + 5 单测）

**Why:** 接口先行，定住 Task 2/3 命令依赖的稳定 DTO 与错误映射（spec §9 code 契约）。按真实 crate API 修正了计划臆造项：`Line=(u64,String)` 行号由调用方提供、计数走 `stats()->SourceStats`、`Frame` 无 `is_extended`（`FrameJson` 不带）、`SignalValue.unit` 为 `String`；`timestamp` 秒→`timestampMs` 毫秒。
**验证:** `cargo test -p lumelog-app can_tests` 5 pass（字段映射/sample.dbc 信号解码含 CoolantTemp raw=90→"Normal"/error 帧空数据/from_can 码稳定/camelCase 序列化）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净（CAN 条目未接入命令前加带注释 `#[allow(dead_code)]`）。

## 2026-10-25 — Plan 3 起草（M2 CanPanel + IPC）

**Files:**
- Create: `docs/plans/2026-10-25-m2-can-panel-ui.md`

**Why:** Plan 2（引擎）已合并 main，起草 M2 另一半的实施计划：`dbc_import`/`can_parse` IPC 命令 + CanPanel 报文表格（列/虚拟滚动/视图过滤/与编辑器双向联动、行号对齐）。导出三件套按 spec §12 留 M3。
**验证:** 纯文档，无代码变更；已按 writing-plans 自审（spec 覆盖 / 签名一致 / 类型一致 / Review Focus 落钉）。计划待用户审阅后执行。

## 2026-10-25 — M2 CAN 引擎（feat/m2-can-engine）

**依据：** spec `docs/specs/2026-10-24-lumelog-v1-design.md` §7；计划 `docs/plans/2026-10-25-m2-can-engine.md`（只做纯 Rust 引擎，IPC/CanPanel 属 Plan 3）。

### 1. lumelog-can 数据模型 + 错误 + workspace 收编
**Files:**
- Modify: `crates/lumelog-can/Cargo.toml`（deps `regex`/`thiserror`（workspace），dev-dep `proptest`，与 lumelog-core 同构）
- Create: `crates/lumelog-can/src/error.rs`（`CanError` 五变体 + `ipc_code()` 钉 spec §9 code 契约）
- Create: `crates/lumelog-can/src/dbc.rs`（`ByteOrder`/`Mux`/`Signal`/`Message`/`Database` 数据模型，解析器 Task 2）
- Create: `crates/lumelog-can/src/frame.rs`（`Frame`/`FrameFlags`/`SignalValue`/`DecodedFrame`/`SourceStats`）
- Create: `crates/lumelog-can/src/lib.rs`（先挂 `error`/`dbc`/`frame`；`decode`/`source` 由 Task 3/4 各自挂入）

**Why:** 先定住后续任务依赖的稳定签名（接口先行），本任务不含算法。
**验证:** `cargo test -p lumelog-can` 2 pass（crate_links + ipc_codes_are_stable）+ `cargo clippy -p lumelog-can --all-targets -- -D warnings` 净。修正：`Frame` 含 `f64` 时间戳→只 derive `PartialEq` 不 derive `Eq`。偏离计划一条：proptest 直接入 crate dev-deps（对齐 core）而非 workspace deps。

### 2. DBC 子集解析器
**Files:**
- Create: `crates/lumelog-can/src/dbc/parse.rs`（严格 `parse` 首个坏构造即中断；宽容 `parse_lenient` 记录行号+上下文+kind 并继续。正则驱动行式解析 `BO_`/`SG_`（含 mux `M`/`m<n`、Intel/Motorola、有符号、factor/offset/min/max/receivers）/`VAL_`（延迟回填 value_table）/`VERSION`/`BU_`；`NS_`/`BS_`/`CM_`/`BA_*`/`VAL_TABLE_` 识别并跳过；未知非空行忽略）
- Modify: `crates/lumelog-can/src/dbc.rs`（`pub mod parse;` + re-export）
- Create: `crates/lumelog-can/tests/data/sample.dbc`（含一条故意坏 `SG_`（行 14））
- Create: `crates/lumelog-can/tests/dbc_golden.rs`（6 例：结构/大小端有符号精确值/注释属性跳过/枚举/mux 标记/宽容跳坏行且严格在该行报 DbcSyntax）

**Why:** spec §7.1 的 DBC 子集；严格/宽容两路对应“跳过坏行继续”；`BO_` dlc 与节点间无逗号。Review Focus 4 钉住。
**验证:** `cargo test -p lumelog-can` 8 pass（2 单测 + 6 dbc_golden）+ `cargo clippy -p lumelog-can --all-targets -- -D warnings` 净（引入 `type ValEntry` 消 type_complexity）。

### 4. FrameSource trait + candump ASCII 源（先于 Task 3 执行：无外部工具依赖）
**Files:**
- Create: `crates/lumelog-can/src/source/mod.rs`（`FrameSource` trait（`next_frame`/`stats`）+ `Line=(u64,String)` + 共享 `hex_to_bytes`）
- Create: `crates/lumelog-can/src/source/candump.rs`（`CandumpSource<I: Iterator<Item=Line>>`：`(ts) iface id#data`、无ts→None、iface 尾数字→channel、`#R`→remote、`##<flags 2hex><data>`→FD+brs/esi+长数据、扩展 id；非hex/garbage/error 行跳过计数，空行不计）
- Modify: `crates/lumelog-can/src/lib.rs`（`pub mod source;`）
- Create: `crates/lumelog-can/tests/data/candump_basic.log` + `crates/lumelog-can/tests/candump.rs`（8 例）

**Why:** spec §7.3.1 candump 形式；Review Focus 3（FD/RTR/error 行不污染流、不 panic）。`next_frame` 拉式设默认行为：遇错行内部推进并计数，直到出帧或 EOF。
**验证:** `cargo test -p lumelog-can` 16 pass（+8 candump）+ `cargo clippy --all-targets -- -D warnings` 净。修正：测试里扩展帧 hex 字面量位数写错（`0x1FF_FFFF`→`0x1FFF_FFFF`，解析器无错）；`next_frame` 由 `while let` 改 `for ... by_ref()` 满足 clippy。

### 5. Vector ASC 源（先于 Task 3 执行：无外部工具依赖）
**Files:**
- Create: `crates/lumelog-can/src/source/asc.rs`（`AscSource<I: Iterator<Item=Line>>`：`<base> <chan> <ID>x? <Rx|Tx> <d|r> <dlc> <b0>...`；token0 parse f64 作 timestamp、id 尾 `x`→去 x 按 hex 解析、`d`→逐 token hex 字节（dlc=实际字节数，>64 跳过）、`r`→remote（dlc 取声明值）；`boe`/`bcs`/`error`/header（`date`/`base`）/非帧行跳过计数，空行不计）
- Modify: `crates/lumelog-can/src/source/mod.rs`（`pub mod asc;` + `pub use AscSource`）
- Create: `crates/lumelog-can/tests/data/asc_basic.asc`（date头/空行/普通帧/扩展帧x/remote/boe/base/CAN前缀，decoded=3 skipped=4）+ `crates/lumelog-can/tests/asc.rs`（6 例）

**Why:** spec §7.3.2 Vector ASC 形式；Review Focus 3（ASC 变体帧/非帧行不污染流、不 panic）。**已知限制**：ASC 方言众多，本实现只保证 spec 描述形态 + 夹具；未覆盖变体（如 `CAN` 前缀行、`boe`/`bcs` 状态行、绝对/相对时间基准换算）作跳过处理，date 头不作时间基准解析。短行安全访问用 `toks.get(..)` 杜绝越界 panic。
**验证:** `cargo test -p lumelog-can` 22 pass（+6 asc）+ `cargo clippy -p lumelog-can --all-targets -- -D warnings` 净。

### 6. 通用正则源（先于 Task 3 执行：无外部工具依赖）
**Files:**
- Create: `crates/lumelog-can/src/source/regex_source.rs`（`RegexSource<I: Iterator<Item=Line>>`；`new(lines, pattern) -> Result<Self, CanError>`：`Regex::new` 失败→`RegexCompile`，`capture_names()` 缺 `id`/`data` 命名组→`BadRegex`（`ts` 可选）。`parse_line` 为自由函数（避开 `self.lines.by_ref()` 与 `&self` 借用冲突）；id 容忍 `0x/0X` 前缀、data 剔除空格/`:`/`,`/`-` 后走 `hex_to_bytes`、dlc=字节数（>64 跳过）、ts→f64）
- Modify: `crates/lumelog-can/src/source/mod.rs`（`pub mod regex_source;` + `pub use RegexSource`）
- Create: `crates/lumelog-can/tests/regex_source.rs`（7 例：含/缺命名组/编译错/自研格式解码/不匹配行跳过/dlc 由 hex 长度驱动/奇数或非 hex 跳过）

**Why:** spec §7.3.3 通用正则源；Review Focus 5（缺 `id`/`data` 构建期即报错；行不匹配跳过计数；data hex 长度驱动 dlc）。修正：regex 1.13 无 `capture_index`→改 `capture_names().flatten()`；`unwrap_err()` 需 `T: Debug`→给 `RegexSource` derive Debug。
**验证:** `cargo test -p lumelog-can` 29 pass（+7 regex）+ `cargo clippy -p lumelog-can --all-targets -- -D warnings` 净。

### 3. 信号位序解码 Intel/Motorola（在 4/5/6 之后执行：需 cantools）
**Files:**
- Create: `crates/lumelog-can/src/decode.rs`（`extract_raw`（Intel LSB-first `start_bit+k`；Motorola DBC 斜线位序：start_bit 作 MSB，字节内向下走、跨到下一高字节 MSB）、`raw_to_signed`（位移符号扩展）、`to_physical`（raw*factor+offset）、`decode_signals`（含 value_table→enum_text））
- Create: `crates/lumelog-can/tests/data/bitorder.dbc`（4 Intel + 4 Motorola 信号，跨字节 len20、有符号负值、factor≠1/offset≠0）
- Create: `crates/lumelog-can/tests/data/golden_bitorder.json`（**cantools 39.4.5 生成的 8 条权威 golden**）+ `crates/lumelog-can/tests/gen_golden.py`（dev 脚本，不进 CI）
- Create: `crates/lumelog-can/tests/bitorder_golden.rs`（手工解析 golden JSON、逐条对拍 raw 精确 + physical 1e-9 容差；无 serde）、`crates/lumelog-can/tests/decode_proptest.rs`（4 个 proptest：Intel 对独立参考式/宽度不溢出/补码往返/物理线性）
- Modify: `crates/lumelog-can/src/lib.rs`（`pub mod decode;`）

**Why:** spec §7.2/§13.1；Review Focus 1（Motorola `@0` 锯齿位序）+ 2（有符号补码 + factor/offset 浮点）。**位序算法经 cantools 39.4.5 golden 逐位对拍通过**（Intel/Motorola × 有/无符号 × 跨字节 len20 × 负值均匹配）。golden 为权威，实现向其靠拢（本次一次即中，未改夹具）。
**验证:** `cargo test -p lumelog-can` 36 pass（+3 bitorder_golden +4 proptest）+ `cargo clippy -p lumelog-can --all-targets -- -D warnings` 净。cantools 仅供开发期生成夹具，仓库已提交 golden、后续无需 Python。

### 7. lumelog-can 解码 CLI（M2 引擎验收）
**Files:**
- Create: `crates/lumelog-can/src/bin/lumelog-can.rs`（手工 `std::env::args` 解析、无 clap/serde；子命令 `dbc --file [--lenient]` 输出 `{messages,signals,errors}`、`decode --log --format candump|asc|regex [--regex] [--dbc] [--limit N=20] [--out jsonl|csv]`，`--dbc` 按 can_id 匹配消息并附 `decode_signals` 物理值；默认输出前 limit 帧 + `{decoded,skipped}`；退出码用法错=2、文件/解析错=1）
- Create: `crates/lumelog-can/src/tests_common.rs`（`pub mod tests_common`：`gen_candump(n, seed) -> Vec<GenFrame>` 确定性 LCG 合成 11-bit 标准帧，供 10万帧验收+基准）
- Modify: `crates/lumelog-can/Cargo.toml`（`[[bin]]`）、`crates/lumelog-can/src/lib.rs`（`pub mod tests_common;`）
- Create: `crates/lumelog-can/tests/data/cli_intel.log`（`can0 64#DEADBEEF12345678`→id100 IntelMsg）+ `crates/lumelog-can/tests/cli_can.rs`（7 例：用 `CARGO_BIN_EXE_lumelog-can` 跑真实二进制；dbc 计数/decode 计数带采样/带 dbc 信号物理值与 golden 一致/无参退2/坏 format退2/缺文件退1/decodes_100k_frames_faithfully）

**Why:** spec §7.5 导出 + §12 M2 验收引擎侧。**10 万帧合成 candump 忠实解码**：decoded==100_000、skipped==0、逐帧 id/dlc/data 与生成端一致；吞吐 debug 档 209.8ms (~2.1 µs/帧，release 会更快；UI 流畅 P4 属 Plan 3)。带 dbc 解码物理值与 Task 3 cantools golden 一致（I_S16 raw -16723→physical -8371.5）。
**验证:** `cargo test -p lumelog-can` 45 pass（+2 tests_common +7 cli_can）+ `cargo clippy -p lumelog-can --all-targets -- -D warnings` 净。修正：测试字符串转义多反斜杠致不闭合；clippy manual_map→改 `.map`。

**M2 引擎汇总：** Plan 2 七个任务（数据模型/DBC 解析/位序解码/candump/ASC/正则/CLI）全部完成；引擎侧验收达标（10万帧正确 + golden 对拍）。IPC 命令层与 CanPanel 属 Plan 3。

### 终审修复（CodeReview 全分支审查 → 两条 Important，“绝不 panic”硬约束）
**Files:** `src/dbc/parse.rs`（`build_signal` 拒绝 length==0 或 >64）、`src/decode.rs`（`extract_raw` 入参防御 `length.min(64)` 钳位）、`src/source/asc.rs`（remote 声明 dlc>64 → 跳过）。
**Why:** 终审发现畸形 DBC `SG_ ...|65@`（移位 ≥64 debug panic）与 ASC remote `Tx r 200`（下游 `data[..dlc]` 越界 panic）两处 panic 路径，违反 Global Constraint“绝不 panic”。三处各加一道守卫（parse 拒范围 + decode 钳位为双保险 + asc 限幅），新3 个回归测试钉住（`signal_length_out_of_range_is_rejected`/`overlong_signal_does_not_panic`/`remote_dlc_over_64_is_skipped`）。
**验证:** `cargo test -p lumelog-can` 48 pass（+3 回归）+ `cargo clippy --all-targets -- -D warnings` 净。

## 2026-10-24 — 界面参考 NoteLume：完整设计系统（feat/m-ui-notelume）

**依据：** 用户指令「界面参考 https://gitee.com/hackerSnail/note-lume 」，经确认采纳「完整设计系统 + 三套主题」。NoteLume 与 LumeLog 同为 Tauri v2 + Rust + Vite + CodeMirror 6 无框架，取其视觉设计语言（非 Markdown 功能）。

### A. 主题基座
**Files:**
- Create: `app/frontend/src/theme.ts`（`resolveTheme`/`nextPref` 纯函数 + `loadPref/savePref/paint/initTheme` 副作用层；三主题 light/dark/vampire + auto 跟随系统；`localStorage` 持久化；`matchMedia` change 重绘）
- Create: `app/frontend/src/theme.test.ts`（`resolveTheme` 显式预设直通 / auto 随系统；`nextPref` 循环序 light→dark→vampire→auto→light）
- Modify: `app/frontend/src/styles.css`（`:root[data-theme]` 三主题 token 层，浅/暗/Dracula 调色板取自 NoteLume；`--font-ui`（Inter/鸿蒙/思源黑，不用宋体）/`--font-mono`（JetBrains Mono/Fira Code）；主题化细滚动条；`box-sizing`；`color-scheme`；现有硬编码配色全迁变量；用 token 给 CodeMirror chrome（gutters/content/cursor/active-line/selection）着色）
- Modify: `app/frontend/src/main.ts`（模块早期 `initTheme()` 绘制；工具条加「主题」循环按钮）

**Why:** M1 交付的是功能骨架，`claude.md` 「自定义主题」仍为待办；先建立可切换的三主题 token 体系作为外壳重构的地基。
**验证:** `npx vitest run` 14 pass（theme 4）+ `tsc --noEmit` 净 + `vite build` OK（styles.css 5.01KB）。

### B. 应用外壳（侧栏 + 检查器 + 可拖分栏）
**Files:**
- Create: `app/frontend/src/shell.ts`（`clampWidth` 纯函数 + `makeResizable` 分栏拖拽（pointer 捕获/钳位/持久化）+ `loadFlag/storeFlag` 收放态持久化 + MIN/MAX 宽度常量）
- Create: `app/frontend/src/shell.test.ts`（`clampWidth` 区间/越界/NaN/锁定；`loadFlag` 无存储回落默认）
- Modify: `app/frontend/index.html`（预绘制内联主题脚本防 FOUC；根容器 `#root`→`#app`；`lang="zh"`）
- Modify: `app/frontend/src/styles.css`（容器选择器 `#root`→`#app`；新增 `.workspace`/`.sidebar`/`.inspector`/`.center`/`.pane-resizer`/`.panel-title`/`.side-item`/`.insp-*` 布局与主题化样式，维持有界视口确定高度链）
- Modify: `app/frontend/src/tabs.ts`（`TabStrip.list()` 按标签序枚举已打开文档）
- Modify: `app/frontend/src/main.ts`（`layout()` 重构为工具条 + workspace（侧栏|拖栏|center(#tabs+搜索面板)|拖栏|检查器）+ 状态栏 + toast；新增 `refreshSidebar/updateInspector/refreshPanels/toggleSidebar/toggleInspector/mkBtn`；激活/关闭/切编码/列表窗口失效时刷新面板；侧栏列已打开文档，检查器展示模式/编码/行尾/总行数/已索引/⚠无法映射/路径）

**Why:** 对齐 NoteLume 的三栏外壳（左导航 + 右检查器）；日志场景侧栏用实时的「已打开文档」列表（前端 `docStates`/tabs 本就权威，不新增后端 IPC；真正的最近文件需 `doc_list` 持久化，归后续）。检查器集中呈现 §5.4 ⚠ 详情。
**验证:** `npx vitest run` 19 pass（shell 5）+ `tsc --noEmit` 净 + `vite build` OK（css 6.58KB）。

### C. 命令面板（Ctrl+Shift+P）
**Files:**
- Create: `app/frontend/src/palette.ts`（`Command` 接口 + 纯函数 `fuzzyScore`（大小写不敏感子序列，前缀/连续/紧凑加分，非子序列返 null）+ `filterCommands`（空查询按原序全列、否则按分降序））
- Create: `app/frontend/src/palette.test.ts`（8 例：空查询=0 / 非子序列=null / 大小写 / 前缀胜于分散 / 排序 / 全列 / 空结果 / 子集）
- Modify: `app/frontend/src/main.ts`（`buildCommands()` 命令集：打开/保存/另存/搜索/跳行/关闭/侧栏/信息/主题×4；`openPalette()` 覆盖层：模糊过滤 + ↑↓/Enter(IME 安全)/Escape + 背板点击关闭 + 选中高亮）；`registerShortcuts` 新增 Ctrl+Shift+P
- Modify: `app/frontend/src/styles.css`（`.palette-overlay` 居中覆盖 + `.palette-box/input/list/row/hint` + 选中 accent + 空态“无匹配命令”）

**Why:** 对齐 NoteLume 的命令面板；把已实现的打开/保存/另存/跳行/搜索/主题/面板收放聚合为可模糊检索的统一入口。评分/过滤为纯函数可 headless 测（与 listview/ipc 同构）。未重复绑 Ctrl+Shift+S（已为另存），侧栏/信息切换走工具条按钮 + 面板。
**验证:** `npx vitest run` 27 pass（palette 8）+ `tsc --noEmit` 净 + `vite build` OK（css 7.58KB）。

### D. 工具条即标题栏（无边框窗口）
**Files:**
- Modify: `app/src-tauri/tauri.conf.json`（主窗口 `decorations: false`，工具条上沿即窗口顶边，对齐 NoteLume 无边框观感）
- Modify: `app/src-tauri/capabilities/default.json`（新增 `core:window:allow-start-dragging/minimize/toggle-maximize/maximize/unmaximize/close/is-maximized` 权限，放行前端自绘窗口控制）
- Modify: `app/frontend/src/main.ts`（`layout()` 工具条改为「应用名 + 动作按钮 | `data-tauri-drag-region` 弹性拖拽区 | 窗口按钮」；`mkWinBtn` 最小化/最大化/关闭；`winAction` 经 `getCurrentWindow()` 驱动且 try/catch 回落 toast；`detectChrome()` 仅在 Tauri 宿主加 `has-custom-titlebar` 并按 UA 标 `data-tauri-platform`）
- Modify: `app/frontend/src/styles.css`（`.titlebar-drag` flex-1 拖拽区；`.win-btns` 默认隐藏、`html.has-custom-titlebar` 才显；`.win-btn` 扁平 42×30 + hover `--bg-hover`、`.close:hover` `--danger`；移除失效 `.appbar .spacer`）

**Why:** 对齐 NoteLume 无边框、工具条兼作标题栏的一体化顶栏。窗口按钮仅在真实 Tauri 宿主出现（浏览器 `vite dev` 无宿主时 `getCurrentWindow()` 抛错 → 回落 toast，按钮也隐藏），拖拽只落在中间弹性区，两侧按钮不受 `data-tauri-drag-region` 干扰仍可点击。
**验证:** `cargo check -p lumelog-app` 净 + `npx vitest run` 27 pass + `tsc --noEmit` 净 + `vite build` OK。拖拽/双击最大化/三按钮的窗口交互属 GUI 手动冒烟（`npm run tauri dev`），headless 不可测。

### E. 收尾与全量验证
**Files:**
- Modify: `docs/claude.md`（功能清单拆分并勾选：「自定义主题」→ 已完成（三主题 + 持久化 + 无边框标题栏 + 命令面板）；「快捷键配置同步」单列为待办）

**Why:** UI 重构 `feat/m-ui-notelume` 四任务（主题基座 / 应用外壳 / 命令面板 / 工具条即标题栏）全部交付，回写功能清单与实况一致。
**验证:** `cargo test --workspace` 全绿（lumelog-core 58 单测 + chunk/cli/docfile/encoding 集成测试）+ `cargo clippy --workspace -- -D warnings` 净 + 前端 `vite build` OK + `npx vitest run` 27 pass。窗口拖拽/最大化/关闭为 headless 不可测的 GUI 项，留 `tauri dev` 手动冒烟。

## 2026-10-24 — M1 全分支终审 fix wave（final whole-branch review）

**Files:**
- Create: `app/frontend/src/styles.css`（185+20 行有界视口布局）、`app/frontend/src/vite-env.d.ts`（`vite/client` 类型，使 CSS import 过 tsc）
- Modify: `app/frontend/src/main.ts`（`import "./styles.css"`；Ctrl+G 跳行小输入框 + Ctrl+Shift+S 另存走 `save` 对话框；`appPing()` 计时终点；只读/编辑两路 `invalidCount` 常驻 ⚠；`saveActive/afterSave` 携带 `target` 路径重绑；DocState 加 `invalid`）
- Modify: `app/frontend/src/listview.ts`（`loadWindow` 类型加 `invalidCount` + `onInvalidCount` 回调；删 inline height 交 CSS；catch 复位 `renderFirst/Last` 防短路）
- Modify: `app/frontend/src/statusbar.ts`（编码名旁常驻 `⚠ {n}` + `setInvalidCount`，`clear` 复位）
- Modify: `app/src-tauri/src/commands.rs`（`MAX_CHUNK_LINES=20000` + `doc_chunk` 上限守卫）
- Modify: `crates/lumelog-core/src/{error.rs,doc/save.rs}` + `app/src-tauri/src/dto.rs`（`Conflict` 改结构体变体携带 `unmappable`；`save_text` 把 `encode_all` 移到冲突门之前使单确认「知情」；`from_core` 中文 detail 合并两门）
- Modify: `crates/lumelog-core/src/bin/lumelog.rs`（`SIZE_LIMIT` `u64::MAX`→`u32::MAX as u64`，stat 阶段快拒 >4GiB）
- Modify: `.github/workflows/release.yml`（资产上传 `find|while` 改进程替换 + `fail` 标志，任一失败 `exit 1` 使 job 失败）
- Modify: `docs/test-checklist.md`（P1 门槛回写 spec 原值 ≤800ms；P1/P2 诚实标 **未验证**；冲突改单确认描述；另存/Ctrl+G 入冒烟；拖放/最近/Save-As 重绑记 §4 deferred）

**Why:** 逐任务门禁因 DOM/交互 headless 不可测而漏掉跨层缝隙；whole-branch 终审暴露 1 Critical + 4 Important + 若干 Minor，一波修完再单点 scoped 复审。

**Findings 处置：**
- **C-1（Critical，修）**：前端原本零 CSS，`.listview` `height:100%` 在 auto 高度祖先里退化 → 撑满 spacer 全高 → `visibleWindow` 一次索取整文件，破 M1 旗舰「1GB 只读滚动」演示。根因修 = styles.css 从 `#root` 起每层给定确定高度；壳侧 `MAX_CHUNK_LINES` 兜底防线；N-4 fetch 失败复位窗口界防卡死。
- **I-1（修）**：plan 钉单 `overwrite_policy` 旋钮耦合冲突门 + 不可映射门；原 `Force` 单确认跨两门 → 用户盲接受不可逆 `?` 覆写。改为 `save_text` 先 `encode_all`（纯、无落盘副作用）再在 `Conflict` 里携带 `unmappable`，前端一次「知情」确认披露全部后果；+回归测试 `conflict_reports_unmappable_count_too`。
- **I-2（修）**：spec §12 M1 验收明列「另存」、§8 明列 Ctrl+G，原前端缺入口。补 Ctrl+Shift+S 另存（`save` 对话框 + `afterSave` 重绑 tab）+ Ctrl+G 跳行内联小输入框（不用 `window.prompt`，WKWebView 不可靠）。拖放/最近 20 非 M1 项 → defer Plan 2。
- **I-3（修）**：checklist 把 P1 误写 ≤1s（spec 实为 ≤800ms），且 P1/P2 本属 GUI 计时 headless 不可测——诚实标未验证，`app_ping` 接线为 P1 计时终点。
- **I-4（修）**：spec §5.4 要求每块 `invalid_count>0` 时状态栏常驻 ⚠；原只读列表丢弃 invalidCount、编辑模式仅一次性 toast。改为两路都透传 → statusbar 常驻 ⚠，随 tab 切换按 doc 恢复。

**Rulings（残留 defer，非阻塞）：** N-1 async 命令化、N-2 UTF-16 整文件、N-6 CSP、N-9 CI-canary 等 Minor 属打磨/加固，不进本 wave；Save-As 后壳侧 doc 仍绑原文件（编码切换重读原内容）为已知 v1 简化，记 checklist §4 归 Plan 2。

**验证:** `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test -p lumelog-core` + `npx vitest run` + `npm run build` 全绿（见下方 commit）。

## 2026-10-24 — Task 11：M1 集成验收 + CI + 发布物（Plan 1 收尾）

**Files:**
- Create: `docs/test-checklist.md`（三平台 P1–P6 实测记录表 + headless 复跑命令 + GUI 冒烟人工清单 + 发布物 + deferred）
- Create: `.github/workflows/ci.yml`（三平台 matrix：`cargo test --workspace` + `cargo clippy --all-targets -D warnings` + 前端 `npm ci`/`build`/`vitest`；bench 仅 workflow_dispatch 不进 PR 门禁）
- Create: `.github/workflows/release.yml`（tag v* 四 target 打包；publish 双重门控 `workflow_dispatch && inputs.publish=='true'` + `secrets.GITEE_TOKEN`）
- Create: `crates/lumelog-core/src/bin/lumelog-gen.rs`（定种合成日志生成器，与 bench 同源 `tests_common::gen_log`，供 checklist P3/P5 复跑）
- Modify: `app/src-tauri/tauri.conf.json`（`bundle.active=true`；targets nsis/msi/dmg/deb/appimage；icon 列表；`webviewInstallMode=downloadBootstrapper`；identifier `com.lumelog.app`→`com.lumelog.desktop` 避开 macOS bundle 扩展冲突）
- Create: `app/src-tauri/icons/*`（`npx tauri icon lumelog-source.png` 生成全套 png/icns/ico）
- Modify: `docs/claude.md`（「流式加载大文件」`[ ]`→`[x]`，加 M1 交付注）

**Why:** M1 里程碑需把散在各任务的验收收敛成一张可复跑、可门禁的清单 + 三平台 CI + 可发布产物。性能门槛原值锁在 spec §2，checklist 只填实测数字不改门槛。

**Rulings:**
- **测量口径**：P3/P5 记录为 **CLI 全进程 end-to-end**（含进程启动 + 建全行索引 + 命中逐行写 stdout），比 criterion 隔离口径保守仍达标。语料须为已落盘静止文件——刚生成即测曾因读操作与脏页回写争用测得 5.9s/26.4s 虚高，落盘后复测 P3 528/654ms、P5 1513ms（16,948 命中）。
- **图标**：ImageGen 额度不可用，改为提交手绘源图 `lumelog-source.png` + `tauri icon` 生成的平台图标全套；一次性生成脚本（pngjs 深路径依赖）与临时测量 harness 均为 scratch，验收后不入库（`pngjs` 已 `npm uninstall` 还原 package.json/lock）。
- **publish 是 outward 副作用**：release.yml 上传 gitee releases 须维护者手动 dispatch + 批准，不落入自动流，符合 SDD「worktree 外副作用」停止点。
- **流式索引 / GUI 级 P1·P2**：threaded doc_open + 8MB 进度 tick 与冷启动/首屏秒表测属人工 GUI 冒烟，checklist §2/§4 记 deferred（Plan 2/3）；headless P3 已证明索引本身达标，UI 联调不阻塞本计划收尾。

**验证:**
- `npx tauri build`（Windows，二次固化 identifier）成功产出：NSIS `LumeLog_0.1.0_x64-setup.exe` **2.59 MiB** + MSI `LumeLog_0.1.0_x64_en-US.msi` **3.80 MiB**（P6 门槛 ≤15MB ✓）
- 1GB 定种语料 `gen_log(1024)` = 926 MiB / 16,777,217 行；实测填 checklist §1：P3 654ms / 峰值RSS 63MB（index-floor 64MB）· P4 p95=0.230ms · P5 1513ms（criterion 纯扫描 ~580ms）——**P3/P4/P5/P6 全部达标**
- ci.yml / release.yml 语法自检（workflow_dispatch 就位；bench 不进 PR）；macOS/Linux 实测由 CI 产物后补测，不阻塞本计划收尾

**M1 状态：** Task 1–10 review clean 并已合并；Task 11（本任务）首轮评审已过 fix loop，待 scoped 复审收尾。核心引擎（编码/索引/取块解码/保存/搜索）+ CLI + Tauri 壳 + IPC + 前端编辑/只读双视图端到端打通；P4/P5/P6 门槛实测通过，P3 索引 headless 通过（UI 级 P1/P2 归后续 GUI 冒烟）。

### Task 11 CodeReview round 1（CHANGES REQUESTED → fix）

评审范围：449c9ae..4754741。无 Critical；2 Important + 2 Minor，均修：
- **#1（Important）** `release.yml`：publish 仅在 `workflow_dispatch` 下运行，而该事件下 `github.ref_name` 是分支名非版本 tag → 会在 Gitee 建出名为分支的 release。修：新增 `tag` 字符串输入，`TAG: github.event.inputs.tag`，脚本内 `test -n "$TAG"` 空值守卫。
- **#2（Important）** `lumelog-gen.rs`：Windows 路径含 `\` 未转义直拼 JSON（`\U`/`\A` 非法转义）→ 输出非合法 JSON。修：逐字符转义 `\` 与 `"` 后再打印（实测 `C:\Users\...` 现输出 `C:\\Users\\...`）。
- **#3（Minor）** `release.yml` job 名渲染 `matrix.platform`（对象→JSON 串不可读）→ 改 `matrix.platform.target`。
- **#4（Minor）** `release.yml` 缺 `actions/setup-node`，与 ci.yml 不一致 → 补 Node 20 step。

验证：`cargo clippy -p lumelog-core --all-targets --release -- -D warnings` CLIPPY_OK；`cargo test -p lumelog-core --release` 57+9+8+1+1 全绿；`lumelog-gen` 实跑输出合法 JSON 已核。

## 2026-10-24 — Task 10：前端只读虚拟列表（ListView）+ shell mode 分流 + P4 bench

**Files:**
- Modify: `app/src-tauri/src/dto.rs`（`DocInfo` 新增 `mode: String`）
- Modify: `app/src-tauri/src/commands.rs`（`EDIT_THRESHOLD = 200MB` + `mode_str()` helper；`doc_open`/`doc_set_encoding` 返回填 `mode`）
- Modify: `app/frontend/src/ipc.ts`（`DocInfo.mode`；新增 `loadWindow(docId,start,count)`——单次 Channel + `doc_chunk`，处理尾 `\n` phantom 行、`count===0` 空窗特判）
- Create: `app/frontend/src/listview.ts`（`ListView implements TabView` + 纯函数 `visibleWindow`）
- Create: `app/frontend/src/listview.test.ts`（5 个 `visibleWindow` 单测）
- Modify: `app/frontend/src/tabs.ts`（泛化：`TabEntry.handle:EditorHandle` → `view: TabView`，新增 `TabView` 接口 `{docId, root, isDirty(), focus(), destroy()}`）
- Modify: `app/frontend/src/editor.ts`（`EditorHandle` 增加 `root: HTMLElement` = `view.dom`，与 `TabView` 契约对齐）
- Modify: `app/frontend/src/statusbar.ts`（新增 `setReadonly(lines)` → `只读 · 共 N 行`）
- Modify: `app/frontend/src/main.ts`（按 `info.mode` 分流 mountList/mountDoc；`editors`/`lists`/`indexed` 三 map；saveActive/changeEncoding/applyMode/handleClose/syncStatus/jumpToLine 全部双视图分支）
- Create: `crates/lumelog-core/benches/chunk_at.rs` + Cargo.toml `[[bench]]`（P4 bench）

**Why:** spec §4.3/A1——>200MB 文档不进 CodeMirror，走固定行高 20px 的虚拟列表，按需 `doc_chunk` 取窗口（±200 行 overscan、rAF 节流、seq 防并发、64MB 侧 LRU 由 core 承担）。mode 由 shell 单点决定（`mode_str(size)`），前端不再各自持阈值，避免壳/前端漂移。

**Ruling:** 流式索引（开线程 + 每 8MB 进度 tick）依 Task 8 Ruling 归 Task 11 GUI 联调；Task 10 保持同步索引，但 IPC Channel 面（`on_index_progress`）与前端 `indexedLines` + "（待索引…）" 占位路径已就绪，Task 11 只需 shell 侧改线程即可接通。

**验证:**
- `npx vitest run`：10 passed（ipc.test 5 + listview.test 5）
- `npm run build`（tsc --noEmit strict + vite）：30 modules，干净通过
- `cargo clippy -p lumelog-app --all-targets -- -D warnings`：APP_OK
- `cargo test -p lumelog-core`：57+9+8+1+1 passed 全绿（bench 新增未波及）
- **P4 实测**（`cargo bench -p lumelog-core --bench chunk_at`，Windows/SSD）：1GB 文件 1000 次随机 600 行取块，**p50=0.153ms · p95=0.230ms · p99=0.259ms**（门槛 p95 ≤16ms，余量约 70×）；criterion 30k iterations 111µs/iter

**headless-deferred（归 Task 11 checklist）:** `tauri dev` 打开 1GB 日志滚动流畅、GBK 不乱码、Ctrl+G goToLine 实际落位、索引进度条 UI。

### Task 10 CodeReview round 1（CHANGES REQUESTED → fix）

评审范围：c61c902..3796355。无 Critical；2 个 Important 均已修：
- **F1** `listview.ts` render/fetchAndRender：`indexed < first`（窗口全在未索引区，Task 11 流式索引路径）时旧代码走 `paint(first, first, …)` 清空可见区，"（待索引…）"占位不显示——违背 Task 8/10 Ruling"占位路径必须已就绪"。修：fetch 条件加 `needEnd > first` 门控，纯占位窗口直接 paint 渲染占位行；删除 fetchAndRender 的 `needEnd<=first` 早退分支。
- **F2** `ipc.ts` `loadWindow`：核实 core `build_chunk` 确实保留每行原始终止符（CRLF 文件每行尾带 `\r`；编辑模式靠 CodeMirror 换行归一化消化，只读列表是裸 DOM 会残留）。修：剥一层尾部 `\r\n`/`\n` 后再按 `\n` split，并逐行剥尾 `\r`。同时修 M2（`payload as ChunkPayload` 强转→`undefined` 窄化）。
- **M1**（随 F1 一并修）：fetch 落回后 `renderFirst/renderLast` 同步为实际 paint 的窗口，防早退判断与 DOM 错位；cache 区间改为按实际到达数据推导（`res.first + lines.length`），不再声称请求区间（防 EOF clamp 后的幽灵缓存命中）。
- **M3**（构造时多取一次 201 行）、**M4**（readonly 下语法下拉点击静默 no-op）：ledger 记 deferred，归 Task 11 GUI 抛光。

验证：vitest 10 passed；tsc strict + vite build 干净（本轮未触 Rust 侧，clippy/test 沿用 commit 3796355 结果）。

### Task 10 round-1 复审（CLEAN）+ round 2 采纳处方

复审判定 F1/F2/M1/M2 全部 ADDRESSED，无新增 Critical/Important；复审另开 2 条不阻塞 Minor，本轮直接采纳其处方原文：
- **Minor-new1**（`ipc.ts`）：逐行无条件剥尾 `\r` 会吃掉混合 EOL 日志里“行内容以裸 `\r` 收尾且无 LF”的字符。修：改为 `split(/\r\n|\n/)`，只把 `\r` 当 CRLF 终止符的一部分处理，裸 `\r` 作为内容保留。
- **Minor-new2**（`listview.ts`）：`cacheStart = Math.min(first, res.first)` 防御方向反了（map 只装有 `[res.first, gotEnd)`，min 会声称 `[first, res.first)` 也在缓存→幽灵命中画空白）。修：`cacheStart = res.first`。
- 另有一条 pre-existing 观察（在途 fetch 落回覆盖 cache 命中绘制，需再滚一次自愈）+ 失败 fetch 无可见反馈：controller 判定归 Task 11（与流式索引/ResizeObserver 一并接），ledger 已记 deferred。

验证：vitest 10 passed；tsc strict + vite build 干净（30 modules）。

## 2026-10-24 — Task 9：前端编辑模式（CodeMirror 6 + Tab + 状态栏）

- 文件：`app/frontend/src/{ipc.ts,encoding-menu.ts,language.ts,editor.ts,tabs.ts,statusbar.ts,main.ts}` + `ipc.test.ts`；`package.json`（+@codemirror/*、@tauri-apps/plugin-dialog、vitest；+test 脚本）；`package-lock.json` 纳入版本库。
- 为什么：实现 spec §8/§4.3/§9——把 Task 8 的 IPC 命令接成可用编辑界面：打开（对话框）→ ≤200MB 进 CodeMirror 编辑、Ctrl+S 保存（冲突/不可映射→确认框）、编码/EOL/语法下拉、dirty 圆点、多标签、行/列状态；>200MB 路由到占位提示（只读虚拟列表属 Task 10）。
- 实现决策：
  - `ipc.ts`：DTO 接口逐字对齐 `dto.rs` 的 serde camelCase；`IpcError{code,message,detail?}` → 中文文案表（覆盖 8 个引擎码 + `not_found/is_directory/permission_denied/unknown_doc/unknown_request/invalid_argument`）；`call<T>` 将任何 rejection 归一化为类型化 `IpcError`（携 `.zh`）；命令包装按 §9 名称，`Channel` 作最后一参；`loadAllText` 循环 `doc_chunk` 拼接全文。
  - **Ruling（包名修正）**：plan Step1 的 `@codemirror/lang-c`、`@codemirror/lang-ini` 均不存在（npm 404）——CodeMirror 官方无 lang-c/lang-ini。C → `@codemirror/lang-cpp`（lezer-cpp 兼盖 C/C++）；INI → 用 `StreamLanguage` 手写简易 mode（不引第三方，合 spec §8 “简易 mode”）。
  - **Ruling（编码切换）**：壳侧 `doc_open` 无 `set_encoding`/编码参数（自动探测），而 core `DocFile::set_encoding` 尚未暴露为 IPC → v1 用“重开 + 用新标签保存”近似（保留行位属 Task 11 GUI 完善）；专 `set_encoding` IPC + 滚动保留缓至 Task 11。
  - 语法高亮：默认 PlainText + log 风格（时间戳/级别/数字/hex，StreamLanguage）+ JSON/C（lezer）+ 自写 INI；扩展名推断 + 状态栏手动切换。标签名以字符串传入（不直接依赖 @lezer/highlight 作声明依赖；HighlightStyle 用 `t.*`）。
  - 未写 CSS 样式表（DOM 结构就位），视觉打磨 + 拖放打开 + 最近 20 文件菜单 + 冲突/不可映射三选完整弹窗缓至 Task 11 GUI 集成。
- 验证：`npm install`（修正后）→ 成功；`npx vitest run` → 5 passed（`ipc_error_code_maps_to_zh_text` 含 8 码全覆盖 + detail 插值 + 未知码回退；`encoding_menu_labels_match_spec` 9 显示名逐个断言）；`npm run build`（`tsc --noEmit` strict + vite）→ 29 模块转换成功，0 error。Step4 `npm run tauri dev` GUI 手测 headless 不可行 → 依 plan 缓至 Task 11。core/app Rust 未动（Stage B 仅前端）。

### Task 9 CodeReview：首轮 SPEC ❌ / Changes requested → fix round 1（2 Critical + 5 Important + 6 Minor + Nit 全修）

首轮 CodeReview（区间 17095e2..064887e）裁定 2 Critical / 5 Important / 6 Minor / 6 Nit。本轮 inline 全修，逐条：
- **C1（必修，spec §4.4）**：`editor.ts` 硬编码 `eol:"LF"` 播种 → 打开 CRLF 文件一次保存即静默改写行尾。修：`doc_open` 于 `state.insert(doc)` 前捕获 `eol_str(doc.eol_style())`，`DocInfo` 新增 `eol` 字段，前端 `eol` 从 `info.eol` 播种（不再硬编码），`docStates.eol` 同步。
- **C2（必修，非可推迟近似）**：编码切换把用户选的标签直接写进 `docStates.encoding` 却不重解码 → 后续 `doc_save` 用错标签静默转码损坏文件，且无 dirty 检查丢改动。修：新增 IPC 命令 `doc_set_encoding`（壳侧 `set_encoding` 清缓存 + `chunk_at` 重读头块 + 返回权威 `DocInfo.encoding`）；前端 `changeEncoding` 走真重解码：dirty 时 `ask` 确认丢弃 → `docSetEncoding` → `loadAllText` → `replaceText`，保存标签恒等于实际解码标签。旧“近似”Ruling 作废。
- **I1**：dirty 圆点不更新——`tabs.refreshDirty` 未写圆点文本。修：`e.dot.textContent = dirty ? "●" : ""`，并在 `openEditable` 接 `onDirty` 首次编辑即刷新。
- **I2**：`setMode` 用 `setState` 重建 state → 丢 undo/scroll 且误置 dirty。修：语言放入 `Compartment`（`@codemirror/state`），`setMode` 改 `view.dispatch({effects: langConf.reconfigure(...)})`，不置 dirty。
- **I3**：高亮有 tag 无颜色（lang-* 只 emit tags）→ 实际不着色。修：`editor.ts` 挂 `syntaxHighlighting(defaultHighlightStyle,{fallback:true})` 与语言并存；`language.ts` 自写 log/INI 用 `HighlightStyle.define` 内联 color，时间戳 token 只吃到末尾（不再 skipToEnd 吞掉同行级别/数字）。
- **I4**：`window.confirm` 在 WKWebView/webkit2gtk 不可靠（常返 false）。修：保存冲突/不可映射、编码切换、关闭脏文档一律改用 `@tauri-apps/plugin-dialog` 的异步 `ask()`。
- **I5**：文档从不释放 → 壳侧 `DocFile`（索引+缓存）泄漏。修：新增 IPC `doc_close`（`state.remove(doc_id)`）；关闭标签、>200MB 拒开路径均调 `docClose`。
- **Minor**：M1 `ipcErrorZh` 去多余“：”且 detail 不丢 + 补 `unknown` 码；M2 非结构化 rejection 归 `{code:"unknown"}`；M4 `tabs` 删死码 `list/closeAll/refreshActiveDirty`，空标签时 `onActivate(-1)`；M5 保存报告 `replaceCount>0` → toast；`encoding-menu.ts` 删死码 `inferEncoding/isSupportedEncoding/EolChoice/EXT_TO_ENCODING`（`EOL_OPTIONS` 作单一真值，`statusbar` 复用）；totalLines 保存后按 `doc.lines` 刷新。
- **API 现实订正（Channel.release）**：首轮曾按“Channel 泄漏”加 `ch.release()`，但本版本 `@tauri-apps/api` 的 `Channel` 无 `release()`——命令返回时 Rust 侧 drop 即发 `end` 标记、JS 自动 `unregisterCallback`。移除错误的 `release()` 调用，注释说明自愈语义。
- **search 缺口补齐**：Produces 要求的前端搜索本轮落地——`openSearchFlow`/`runSearch`（`searchRun mode:"literal"`）/`renderSearchPanel`（命中行 + snippet + 点击 jumpToLine + 取消/关闭）+ `Ctrl+Shift+F`。
- 验证（fix round 1）：`npx vitest run` → 5 passed；`npm run build`（tsc strict）→ 29 模块 0 error；`cargo clippy -p lumelog-app --all-targets -- -D warnings` → 干净；`cargo test -p lumelog-core` → 57 lib + 9 cli(+1 ignored) + 8 docfile + 2 proptest 全 passed。GUI 手测/GBK 往返/实际高亮渲染仍 headless 不可验 → 依 plan 缓至 Task 11。

### Task 9 fix round 1 复审 → 新增 search 流的 fix round 2（仅修 search）

round-1 scoped 复审裁定：C1/C2/I1-I5/全部 Minor **均 ADDRESSED**，无回归；唯一 CHANGES REQUESTED 集中在本轮**新增**的前端 search 流：
- **Important（新）**：“取消”按钮不可用——`search_run` 是同步命令，整段扫完才 `Ok(req_id)`，`lastReqId` 在 `await` 前恒为 null → 点取消为空操作（与 Task 8 F2 同源）。**Ruling**：plan Task 9 Produces 明定搜索“仅展示命中列表与 snippet（Plan 3 完善联动）”，取消属异步 search（Task 10，与 Task 8 F2 一致）——本轮不ship 一个坏死控件：移除“取消”按钮（保留“关闭”），`lastReqId`/`searchCancel` 前端引用一并删除（ipc.ts 仍导出 `searchCancel` 镜像命令面）。
- **Minor M-new1（新）**：命中无上限累加 + 每次整面板重建 O(n²)。修：不保留 hits 数组（`searchTotal` 仅计数），增量 append 行，DOM 硬上限 `SEARCH_RENDER_CAP=1000`（总数以 `onDone.matches` 为准）。
- **Minor M-new2（新）**：`window.prompt` 同 I4 不可靠。修：面板内联 `<input>` + 回车/“搜索”按钮提交，不再用原生 prompt。
- **Minor M-new3（新，核实为非问题）**：空文件切编码是否 `chunk_at(0,1)` 越界——core“empty_file_is_one_line”定空文件 line_count=≥ 1，`0 < 1` 合法→不报 `LineOutOfRange`。不改。
- **Ledger 级清理**：`statusbar.ts` 顶注“reopenWithEncoding/re-opens”陈旧→改“re-decodes in place / changeEncoding → doc_set_encoding”；`runSearch` 死参数 `handle` 删除（jumpToLine 取 active tab）。
- 验证（fix round 2）：`npx vitest run` → 5 passed；`npm run build`（tsc strict）→ 29 模块 0 error（无未用导入/变量）。本轮仅动前端 search + 注释，Rust 壳未变。

### Task 9 fix round 3（仅修常驻 search 面板新引入的两缺陷）

round-2 scoped 复审：4 项 findings 均 ADDRESSED、无回归；但常驻面板改写新引入 2 Important（均为 1–2 行）：
- **Important（新）**：IME 合成中回车误提交——中文拼音候选词回车确认同样 `key==="Enter"`（`isComposing===true`）→ 半截串触发同步扫描。修：`if (e.key === "Enter" && !e.isComposing)`。
- **Important（新）**：面板冻结开面板时的 `docId`——切标签后提交仍扫旧文档，而 `jumpToLine` 作用于当前文档→行号错位静默夹紧。修：`submit` 每次重解析 `tabs.active()`；`onActivate` 改调 `hideSearchPanel()`（陈旧命中行不可点）。
- **Minor（新）**：`onDone` 覆盖“仅显示前 1000”提示。修：`s.matches > SEARCH_RENDER_CAP` 时补回截断提示。
- **顺手**：`openSearchFlow`/`submit` 在 `searchBusy` 时给提示而非静吞/重建致空白面板；`runSearch` 文档已关时 `statusLine` 提示“文档已关闭”。
- 验证（fix round 3）：`npx vitest run` → 5 passed；`npm run build`（tsc strict）→ 29 模块 0 error。仅动前端 search。


## 2026-10-24 — Task 8（阶段 B）：Tauri v2 壳 + IPC 命令层

- 文件：新增 `app/src-tauri/`（`Cargo.toml` 包名 `lumelog-app`、`build.rs`、`tauri.conf.json`、`capabilities/default.json`、`src/{main.rs,lib.rs,state.rs,dto.rs,commands.rs}`、`icons/icon.ico`）；新增 `app/frontend/` Vite vanilla-ts 骨架（`package.json`/`vite.config.ts`/`index.html`/`src/main.ts`/`tsconfig.json` + `dist/index.html` 占位供 `generate_context!`）；根 `Cargo.toml` members 增 `app/src-tauri`。
- 为什么：实现 spec §9 / plan Task 8 的 IPC 命令层——把阶段 A 下沉 core 的纯逻辑（`validate_open_path`/`build_index_with_progress`）转发为 Tauri 命令，前端可调用。7 命令：`app_ping`、`doc_open`（`Channel<IndexProgress>` 进度）、`doc_chunk`（`Channel<ChunkPayload>`，char_map 不出 IPC）、`doc_save`（`IpcError` 码映射）、`doc_list`、`search_run`（`Channel<Vec<MatchJson>>`+`Channel<SearchStatsJson>`+取消）、`search_cancel`。
- 实现决策：
  - `create-tauri-app` 需交互/浏览器 → 手写等价壳（plan Step1 明示“非交互参数按其 CLI 现状调整”，产物与模板同形）。
  - `IpcError { code, message, detail? }` 自定义可序列化错误类型作命令 `Err`：`from_core`/`from_open_reject` 映射码 `io|too_large|conflict|unmappable|locked|line_out_of_range|encoding_unsupported|invalid_pattern|not_found|is_directory|permission_denied`（对齐 plan Produces）。
  - `state.rs`：`AppDocs { docs: Mutex<HashMap<u64, Arc<Mutex<DocFile>>>>, next_doc_id: AtomicU64, cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>, next_req_id }` 挂 `tauri::manage`。
  - **Ruling**：v1 `doc_open` 同步索引（~0.6 s/GB）+ `SIZE_LIMIT=u32::MAX as u64`（与 core `LineIndexBuilder::finish` 的 4GB/u32 硬门逐字节一致，前置零 I/O 快速拒）；后台线程逐 8MB 流式推进留 Task 11 GUI 联调（IPC 面 `Channel<IndexProgress>` 已就绪）。`tauri::generate_handler!` 编译期校验全命令签名，故 IPC 接线在 `cargo check` 阶段即被类型检查。
  - `icon.ico`：tauri-build 的 winres 步骤硬性要求 `icons/icon.ico` → `.superpowers/gen_icon.cjs`（node）合成合法 16×16 32bpp ICO，纳入版本库（构建所需资源）。
- 验证：`cargo check -p lumelog-app` → Finished（build script 通过后真正 typecheck commands/dto/state/lib，tauri v2.12.5 全树 362 包编译通过，无 error/warning）；`cargo clippy -p lumelog-app --all-targets -- -D warnings` 无告警；`cargo test -p lumelog-core` → 57 lib + 9 cli(+1 ignored) + 8 docfile + proptest 全 passed（阶段 B 未波及 core）。Step4 `npm run tauri dev` GUI 冒烟 headless 无 display 不可行 → 依 plan 缓至 Task 11；前端 `npm run build` 属 Task 9 Step1 交付物，本任务不越界。

### Task 8 CodeReview：首轮 SPEC ❌ / Changes requested（1 必修 + 1 Ruling 接受）

首轮 CodeReview（区间 7912c47..4a6985a，Stage A+B）裁定：
- **F1（必修、spec §9 / plan Step 3 合规）**：`doc_open` 的 `SIZE_LIMIT = u64::MAX` 使 >4GB 前置快速拒绝形同虚设——`validate_open_path(p, u64::MAX)` 永不因 size 拒（旁路），`DocFile::open(_, u64::MAX)` 的 `size > size_limit` 门亦旁路，>4GB 文件要完整流式扫到 `LineIndexBuilder::finish` 才报 `TooLarge`（与 spec §9“前置校验…>4GB → 对应错误码弹窗、不记入最近文件”的零 I/O 快速拒意图相左）。修：`SIZE_LIMIT = u32::MAX as u64`——与 `line_index.rs:125` 的 `file_size > u32::MAX as u64` 硬门逐字节一致（拒收集不变），仅把拒绝时机前移到 stat 阶段。修后 `cargo clippy -p lumelog-app --all-targets -- -D warnings` 仍干净。
- **F2（Ruling 接受、缓至 Task 10）**：`search_run` 同步模型下整段扫描完成才 `Ok(req_id)`，前端 `await` 期间无法取到 id 调 `search_cancel`→必返 `unknown_request`。取消机制结构上生命周期完整、无泄漏；commands.rs 模块注释已明记“Heavy calls run as synchronous commands for Task 8”，属 plan Step2 明示的 v1 文档化简化，非遗漏。async/非阻塞索引是 Task 10 大文件 UI 交付，自然修复。
- **裁决（GUI 冒烟 / IPC 无运行时测试）**：评审认 Step4 `npm run tauri dev` headless 不可执行属可接受（编译期 `generate_handler!` 已校签名 + Channel 类型，clippy 干净，plan 将运行时 GUI 验收归 Task 9/10/11）；Stage B commands 无运行时测亦可接受（plan 明文“纯逻辑下沉 core、commands 只做转发”，可测逻辑已由 Stage A 6 测覆盖）。

## 2026-10-24 — Task 8（阶段 A）：IPC 前置校验 + 带进度索引下沉 core

- 文件：`src/error.rs` 新增 `OpenReject`（NotFound/IsDirectory/PermissionDenied/TooLarge，`PartialEq`）；`src/doc/file.rs` 把私有 `build_index` 拆成公共 `build_index_with_progress(file, size, label, chunk_size, on_progress)`（逐块回调累计字节）+ `build_index` 委派（DRY，单一代码路径），新增纯函数 `validate_open_path(path, size_limit) -> Result<(), OpenReject>`；`doc/mod.rs` re-export 两函数；`file.rs` 末尾 `#[cfg(test)] mod ipc_logic_tests`（6 测）。
- 为什么：plan Task 8 Step 2/3 明示“用纯逻辑拆分保可测”——索引线程与 `doc_open` 前置校验的重逻辑放 core（Task 2 已有 `LineIndexBuilder`，包一层），Tauri 命令层只做转发；本阶段为无头可验证部分（不依赖 Tauri），Step 4 的 GUI 冒烟依 plan 缓至 Task 11。
- 实现决策：
  - `on_progress` 用 `&mut dyn FnMut(u64)`（object-safe，供后台线程携带 Channel 闭包）；UTF-16 整缓冲扫描仅在完成时回报一次（与既有全量读路径一致），字节编码每块回报，故 `chunk=8MB` 下逐 8MB 推进（spec §9 `on_index_progress`）。
  - `validate_open_path` 只 stat 路径不开句柄，纯函数；“其他 io 错误”归 NotFound（真正读失败仍由后续 `DocFile::open` 精确上报），`TooLarge` 携带 (实际大小, 上限) 供 IPC 文案。
  - `OpenReject` 与 `CoreError` 分立：IPC 层据此映射专码 `not_found|is_directory|permission_denied|too_large`，而非泛 `io`。
- 验证：`cargo test -p lumelog-core --lib` → 57 passed（+6：`validate_open_path_ok_under_limit`/`_not_found`/`_is_directory`/`_too_large_reports_sizes`、`build_index_with_progress_matches_whole_and_reports_monotonically`（对拍 `line_index::build` 逐字节同 offsets + 严格递增且终止于 EOF）、`build_index_with_progress_single_report_for_tiny_file`）；`cargo clippy -p lumelog-core --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 7: lumelog CLI 冒烟（M0 验收）

- 文件：新增 `src/bin/lumelog.rs`（`open|search|save` 三子命令 + 手工 `std::env::args` 解析 + 手搓 JSON 输出）、`tests/cli_smoke.rs`（7 个断言测试 + 1 个 `#[ignore]` 1GiB 计时冒烟）；数据文件 `tests/data/sample_gbk.log`（GBK 20 行台架日志，由 `ensure_sample` 幂等生成并纳入版本库）。
- 为什么：spec §3「重逻辑放纯 crate、薄 CLI 可复用并冒烟」——把 Task 2–6 的公开接口（DocFile/search_stream/save_text/encoding）串成端到端可执行路径，钉死 M0 验收：`open` 打印 `{size,lines,encoding,eol}` 摘要 JSON、`search` 逐命中 NDJSON + 结尾 stats、`save` 从 stdin 读新文本按磁盘编码重编码落盘；退出码 `0` ok / `1` 运行时错误 / `2` 用法错误（无参或未知子命令）。
- 实现决策：
  - **不引 clap**（plan Step1 明示避免多一个依赖）：`parse` 单遍扫描——flag 取下一个非 `-` token 作值，否则视为布尔 flag；遇裸位置参数即判用法错误退 2。core 主依赖无 serde，故 `open`/`search`/`save` 的 JSON 全部手写格式串 + `json_escape`（转义 `"` `\` 控制字符）。
  - 复用 crate 接口而非重写逻辑：`open_doc` 统一 `--file`→`DocFile::open(SIZE_LIMIT=u64::MAX)`（4GB/u32 上限仍由行索引内部 `TooLarge` 把关）+ 可选 `--encoding` 覆盖；`search` 用 `doc.encoding()` 作查询编码、`--regex`/`--ignore-case`/`--max` 映射到 `SearchQuery`/取消；`save` 先 `read_to_string(stdin)` 再 `disk_meta()`（早于 `save_text` 的 `&mut` 借用）取冲突基线，`--eol LF|CRLF` 覆盖否则沿用探测风格，`OverwritePolicy::SafetyConfirm`。
  - `search` 超 `--max` 时停止打印但继续扫描计数，`stats.truncated` 回报截断。
- 验证：`cargo test -p lumelog-core --test cli_smoke` → 7 passed（`open_prints_summary_json`/`open_line_count_flag_prints_only_count`/`search_counts_literal_gbk_hits`(20 命中)/`save_roundtrips_gbk_from_stdin`(GBK 重编码逐字节一致)/`no_args_exits_with_code_2`/`unknown_subcommand_exits_2`/`missing_file_is_error_code_1`）；全量 `cargo test -p lumelog-core` → 51 lib + 7 cli + 8 docfile + chunk/encoding proptest 全 passed；`cargo clippy -p lumelog-core --all-targets -D warnings` 无告警。
- **P3 参考（`open_1gb_wall_time` --ignored）**：`lumelog open` 打开 971,234,620 字节（16,777,217 行）合成日志，含进程内整体行索引构建，wall **≈ 590 ms**（release）。CLI 路径首屏即返回摘要 JSON，索引一次性 build；UI 级流式首屏验收在 Task 11。

### Task 7 CodeReview：Approved（无 Critical/Important）+ 应用 3 项 Minor polish

首轮 CodeReview（区间 81dc22d..3be2040，含实测退码与夹具字节校验）裁定 **Approved**：规范全达标、无 Critical/Important、5 项 Minor。其中 3 项有实质价值，作为 post-approval polish（Rulings, not stalls）当场修复：
- **Minor-2**（真实缺陷）：`cmd_open` 的 `--head` 分支二次 `open_doc` 重开文件、重建整个行索引只为拿 `&mut` 句柄 → 改为单次 `let mut doc` 复用同一句柄。
- **Minor-3**（退码契约正确性）：缺 `--file`/`--pattern` 等必填 flag 与 `--head abc`/`--max x`/`--eol zzz`/未知 `--encoding` 这类坏值，原借 `CoreError::Encoding` 走 exit **1** 且报“encoding error: missing --file”误导 → bin 新增本地 `CliError{Usage→2, Engine→1}`（`From<CoreError>`/`From<io::Error>` 自动归 Engine），用法错误统一 exit **2** + 打印 usage。补两测 `missing_required_flag_is_usage_exit_2`/`bad_flag_value_is_usage_exit_2` 钉住。`missing_file_is_error_code_1`（文件不存在=运行时错）仍为 exit 1 不变。未改 crate 的 `CoreError`。
- **Minor-4**（测试卫生）：`save_roundtrips_gbk_from_stdin` 子 stdout 继承泄漏一行 JSON 到 `cargo test` 输出 → `spawn` 前 `.stdout(Stdio::piped())`。
- **Ruling**：Minor-1（`parse` 取下一非 `-` token 作值，值以 `-` 开头会误判为 flag）与 Minor-5（`--max` 超限继续扫描 / `let _ = writeln!` 吞 stdout 错）——plan Step1 明示手工启发式解析、且为冒烟 CLI，两者只导致干净报错不影响验收契约，接受不改。
- 验证：`cargo test -p lumelog-core --test cli_smoke` → 9 passed（原 7 + 2）/ 1 ignored；全量 51 lib + 9 cli + 8 docfile + chunk/encoding proptest 全 passed；`cargo clippy -p lumelog-core --all-targets -D warnings` 无告警。commit `polish:lumelog-cli-review-minors`。

## 2026-10-24 — Task 6: 搜索（分块正则 + 重叠去重 + 取消）

- 文件：新增 `src/search.rs`（Mode/SearchQuery/Match/SearchStats + `search_stream`/内部 `search_with_window`/`build_regex`/`make_snippet` + 8 单测）；`doc/file.rs` 加 `pub(crate)` `read_window`/`line_at_byte`/`line_byte_range`（供扫描器按原始字节窗口读 + 字节→行换算）；`lib.rs` 挂 `pub mod search`，并把 `tests_common` 从 `#[cfg(test)]` 改 `pub mod`（criterion bench 目标把本 crate 当外部依赖编译，`cfg(test)` 关闭，需 public 才能用 `gen_log`）；新增 `benches/search.rs` + `Cargo.toml` `[[bench]] name="search"`。
- 为什么：实现 spec §6.2/A8 超大日志流式搜索（按查询编码匹配磁盘字节）+ Review Focus 4（命中跨搜索块边界不漏不重）+ §5.2 行号与编码无关。
- 实现决策：
  - **字节空间扫描**：查询串先按 `q.encoding` `encode_all` 成字节（A8），在原始字节的窗口上跑 `regex::bytes` 匹配——中文 GBK 查询命中磁盘上的 GBK 字节序列（而非其 UTF-8 拼写）。Literal 把每个字节转义成 `\xHH` 并 **`.unicode(false)`**（否则 `\xC4` 被当作标量 U+00C4 匹配成 UTF-8 两字节，实测命中 0）；Regex 保留 unicode 默认，对 UTF-8 文件/ASCII 模式自然逐字节生效。
  - **跨窗口命中不漏不重**：用 committed-region 法——每窗只提交起点落在 `[win_start, win_start+step)`（step=window-overlap）的命中，尾部 overlap 区留给下一窗完整重读；因 OVERLAP≥命中长度，跨接缝命中必在某一窗被完整匹配且仅提交一次（区别于易误删独立命中的单调去重键）。要求单条命中 ≤ OVERLAP(1024)，v1 界（日志查询短），已在模块注释记。
  - UTF-16：编码后查询串含 `0x00` → `CoreError::Encoding("UTF-16 search unsupported in v1")`（大文件 UTF-16 搜索进 Plan 3）。
  - **Ruling**：算法钉 WINDOW=8MB/OVERLAP=1024，但拆出内部 `search_with_window(window, overlap)` 让测试用极小窗口（8/4）在字节级精确钉 seam 不漏不重与取消，免造 >8MB 测试文件；公共 `search_stream` 仅委托常量。
  - snippet：取所在行 `chunk_at(line,1)` 解码文本 + `char_map.byte_to_line_cu` 定命中的行内 cu16，截 ±40 cu16 去尾换行（Ruling：BMP 文本 cu16==char，astral 罕见）。
- 验证：`cargo test -p lumelog-core` → 47 lib passed（search 8：`literal_hits_gbk_chinese`/`regex_overlap_match_not_missed_not_doubled`/`cancel_stops_early`/`line_number_correct`/`stats_counts_all_matches_gb18030_file`/`utf16_query_is_rejected`/`case_insensitive_finds_both_cases`/`regex_mode_alternation_over_utf8`）+ chunk_proptest(500) + docfile 8 + encoding_proptest 全 passed；`cargo clippy --all-targets -D warnings` 无告警。`cargo bench --bench search`（criterion，~0.9GB 合成日志，10 samples，`search_literal_1gb`）：time **[565.99 ms 580.63 ms 595.44 ms]**（P5 目标 <10s 达标，手动记录，不卡 CI）。

### Task 6 修订（首轮 CodeReview 1C+1I+3M，均修复）

首轮 CodeReview（区间 bc97b61..52bb492）裁定 Changes requested：
- **C1** committed-region 每窗固定推进 `cursor = win_start + step`：当提交区内最后一条命中的 **END 越过 committed_end**（跨接缝的密集命中），下一窗从 committed_end 起重扫，与全文件 `find_iter` 的**非重叠续扫点不一致** → 真实命中被丢失、并被伪造命中替换（实测反例：`"AAAAAAAAAB"` 搜 `"AA"`，window=8/overlap=3 得 `{0,2,4,5,7}` 而非参考 `{0,2,4,6}`）。修：发射时记录 `resume = max(committed_end, 最后发射命中 END)`，下一窗 `cursor = resume`（`find_iter` 升序非重叠保证被跳过的尾部命中起点 ≥ resume，故不漏不重）。新增 `dense_matches_across_seam_equal_whole_file`：以单窗（整文件）为 oracle，对 (window,overlap) 网格拍字面与正则密集命中。关联：Regex 无界 pattern（`.*`/`\d{2000}`）可产 >OVERLAP(1024) 命中，seam 处不保证 → v1 界已在模块注释文档化。
- **I1** 最终窗口从不回调 `progress`（`if is_final || !progress(..)` 短路）：小文件首轮即 final → `progress` 一次都不调，UI 进度到不了 100%。修：final 窗也回调一次（返回值忽略，扫描已完成），非 final 窗据返回值置 `cancelled`。
- **M1** 非法正则误归 `CoreError::Encoding` → 新增 `CoreError::InvalidPattern(String)` 专用于 pattern 错误（空/正则语法错/不可表示），UTF-16 越域仍归 `Encoding`。
- **M2** `encode_all` 的 `replace_count` 被忽略：不可表示的字符降级成 `?`，搜它会静默命中所有字面问号。修：Literal 下 `replace_count>0` → `InvalidPattern`。测 `unmappable_literal_pattern_rejected`。
- **M3** 空 pattern 仅 Literal 拦截（Regex 空 → 每字节 0 宽命中，近 DoS）。修：`build_regex` 顶部对两模式统一拒空。测 `empty_pattern_rejected_both_modes` + `invalid_regex_pattern_rejected`。
- 验证：`cargo test -p lumelog-core` → 51 lib passed（search 12：原 8 + `dense_matches_across_seam_equal_whole_file`/`empty_pattern_rejected_both_modes`/`unmappable_literal_pattern_rejected`/`invalid_regex_pattern_rejected`）+ proptest(500) + docfile 8 + encoding_proptest 全 passed；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 5: 保存（原子写/EOL/冲突/不可映射）

- 文件：新增 `src/doc/save.rs`（EolStyle/detect_eol、OverwritePolicy、SaveOptions/SaveReport、save_text、atomic_write）；`doc/file.rs`（DocFile 加 `eol_style` 字段，open 时从首部解码窗口 `detect_eol` 记录；加 `eol_style()` getter 与 `pub(crate) note_saved`）；`doc/mod.rs` 挂 `pub mod save` + re-export。
- 为什么：实现 spec §4.4 保存四件事——行尾风格（打开检测/保存写回、Mixed 写为 CRLF）、原子写（同目录 `.{stem}.lumelogtmp` → fsync → rename；Windows 目标被占 rename 失败降级 `CoreError::Locked`）、冲突检测（SafetyConfirm 比对 disk_meta，脏则不落盘返 `Conflict`，Force 越权写入并回报 conflict）、不可映射（编码 replace_count>0 且非 Force 时先返 `Unmappable(N)` 且磁盘不变，保证“确认后执行”）；Review Focus 3（中文 GBK+CRLF 往返逐行一致）。
- 实现决策：
  - 保存落盘字节 = text 先归一 LF（`\r\n`/单独 `\r` 都→`\n`）再按 `opts.eol` 展开（Crlf/Mixed → `\n`→`\r\n`）；BOM 由 `encode_all` 按 label 自动加（UTF-8 BOM/UTF-16）。
  - **Ruling**：plan 里 detect_eol 描述“CRLF计数≥LF独立计数→Crlf”与“两类都有→Mixed”矛盾→取无歧义的后者：两类均现→Mixed，仅一种→该风格，无换行→LF（写入默认）。
  - **Ruling**：冲突仅在“覆写原始路径”时有意义（opts.path==doc.path 或 None）；另存新路径不比冲突（磁盘无基线可比）。写入成功后 `note_saved` 刷新 doc 的 size/mtime/eol，使紧接的再保存不会误报冲突。
  - rename 失败归 `Locked` 的判据：`kind()==PermissionDenied` 或 Windows raw_os_error ∈ {5 访问拒绝, 32 共享冲突}；失败时 best-effort 删临时文件。
- 验证：`cargo test -p lumelog-core` → 32 lib passed（含 save 6 个：`detect_eol_cases`/`save_roundtrip_gbk_crlf`/`conflict_blocks_save_without_force`/`unmappable_reports_count_before_write`/`atomic_write_leaves_no_tmp_on_success`/`save_as_to_new_path_is_not_a_conflict`）+ chunk_proptest(500) + docfile 8 + encoding_proptest 全 passed；`cargo clippy --all-targets -D warnings` 无告警。

### Task 5 修订（首轮 CodeReview 发现 3C+4I，均修复）

首轮 CodeReview（区间 f8cde0a..b12f91b）裁定 Changes requested：
- **C1** 就地保存后 DocFile 半更新：`atomic_write` 的 rename 替换了目录项，但旧 `File` 句柄仍指向被丢弃的旧 inode、旧行索引偏移失效 → `chunk_at`/搜索会读到**保存前的旧内容**。修：新增 `DocFile::reload_after_save(encoding)`，就地保存成功后重开句柄 + 从新字节重建行索引 + 按新编码重探 EOL（Save-As 不触发）。补测 `save_inplace_reindexes_after_transcode` + `atomic_write` 新增“保存后 chunk_at 读到新内容”断言。
- **C2** 冲突门用裸路径字符串相等判 `overwriting_original`，大小写/链接/异写法同名文件会绕过冲突检查 → 静默覆盖。修：`same_file()` 先 `canonicalize` 再比，失败回退词法。写目标也 `canonicalize`（同时解 I4 软链）。
- **C3** 不可映射写出的实际字节：`encoding_rs` 批量 lossy `encode` 对不可映射字符输出 HTML 数字实体 `&#128169;`（9 字节），而非 spec §4.4 承诺的替换符，且会改写行内容/长度、破坏往返。修：`encode_all` 逐码位编码，不可映射时写单个占位符（GBK/BIG5/Shift-JIS/ISO-8859-1 无 U+FFFD 码位→`?`；GB18030 可表全部 Unicode→不触发），不再出现数字实体。C3a（单一 `overwrite_policy` 同时豁免两门）= **Ruling**：plan 钉单字段，UI(Task8/9) 需分别确认后才传 Force，已在枚举文档说明。
- **I1** Save-As 也调 `note_saved` 污染原文件冲突基线 → 改为仅 `overwriting_original` 时 reload；`save_as_to_new_path` 补 `doc.disk_meta()==stale` 断言钉住。
- **I2** 写阶段（create/write/fsync）失败不清理临时文件（与函数文档矛盾）→ 拆 `write_stage`，任一步失败 best-effort 删 tmp；新增 `write_failure_removes_tmp`（目标传目录使 rename 失败）。
- **I3** `is_locked` 将非 Windows `PermissionDenied` 误判为占用 → 用 `#[cfg(windows)]` 门控（5/32），unix 下权限问题归 `Io`。
- **I4** 原子写副作用：软链被替换为普通文件/未 fsync 父目录 → 写通过 `canonicalize` 落真实路径 + `#[cfg(unix)]` rename 后 fsync 父目录；属性保留缓至后续。
- **M1** `Mixed` 写盘后 `eol_style` 仍记 Mixed → 现由 `reload_after_save` 从实际字节重新 `detect_eol`（自然 settle 为 Crlf）；**M3** `detect_eol` 不识别裸 `\r` 已在文档注释说明。
- 测试补齐：Mixed/裸 CR 展开、UTF-8 BOM 保存往返+重探、post-save chunk_at 新鲜、rename 失败 tmp 清理、Force 写占位符非数字实体、Save-As 不改 doc 基线。GBK emoji 不可映射现断言写为单字符 `?` 且不含 `&#`（`encode_all` 行为修正同步反映到 encoding 单测）。
- 验证：`cargo test -p lumelog-core` → 37 lib passed（save 11 个）+ proptest(500) + docfile 8 + encoding_proptest 全 passed；`cargo clippy --all-targets -D warnings` 无告警。二次 scoped re-review（区间 b12f91b..866a2e0）裁定 Approved（附 1 项 SHOULD FIX）。

### Task 5 二次评审 polish（性能与测试硬化）

二次 scoped re-review Approved，逐条确认首轮 3C+4I+M 均修复、无功能回归；用 encoding_rs 0.8.42 源码证明可表示字符的逐码位字节输出与批量编码逐字节等价（编码器无跨字符状态，仅 Iso2022Jp 有 pending）。以下为 Approved 后处理的可修项：
- **W-1**（SHOULD FIX，性能回归）：C3b 的逐码位 `encode_all` 会给每个非 ASCII 字符带来一次堆分配（5MB GBK ≈ 170 万次，慢 20–40×），对中文日志核心负载是实质回归。修：先跑一次批量 `enc.encode(text)`，`had_errors==false`（绝大多数）直接返回批量结果（一次分配）；仅当确有不可映射时才回退逐码位写 `?`。可表示字符的输出与原批量逐字节一致。
- **N-1**（tmp 碰撞）：`temp_path` 原为固定名 `.{stem}.lumelogtmp`，同目录同词干文件（`a.txt`/`a.md`）或并发多 Tab 保存会抢同一临时文件互相截走。改为 `.{stem}.{pid}.{seq}.lumelogtmp`（`static SEQ: AtomicUsize`）。
- **S-3**（诚实性）：`SaveReport.replace_count` 与 `save_text` 内注释从“写 U+FFFD”订正为“写单个 `?` 占位符”（与 C3b 实际行为一致）；`encode_all` 的 GB18030 注释补 U+E5E5 例外。补记：初版的 `note_saved` 已被 `reload_after_save` 取代（见 C1/I1）。
- **S-1/S-2**（测试硬化）：`write_tmp` 改为每次建独立子目录避免并行测试互扰；新增 `stray_tmps(dir)` 扫描 `*.lumelogtmp`（因 N-1 随机 tmp 名，旧 `!temp_path(&p).exists()` 断言变恒真，改用扫描才真正钉住“无残留”）；`write_failure_removes_tmp` 重命名为 `rename_failure_removes_tmp`（诚实标注其覆盖 rename-Err 分支）；新增 `write_stage_failure_returns_error_and_creates_nothing`（create/write 分支）与 `save_as_to_nonexistent_path_creates_it_without_conflict`（canonicalize 失败回退 + Save-As 不污染基线）。
- 验证：`cargo test -p lumelog-core` → 39 lib passed（save 13 个）+ proptest(500) + docfile 8 + encoding_proptest 全 passed；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 4: DocFile 取块解码 + char_map

- 文件：新增 `src/doc/chunk.rs`（Chunk/CharMap/LogicalLine + build_chunk，含 GB18030 误切合并重试）、`src/doc/file.rs`（DocFile：open 探测编码+建索引 / set_encoding / chunk_at / 手写 VecDeque 64MB LRU / UTF-16 走 build_aligned 偶数对齐索引）、`doc/mod.rs` re-export；`tests/chunk_proptest.rs`（byte_to_line_cu 对齐 oracle）、`tests/docfile.rs`（临时文件集成）。
- 为什么：实现 spec §4.2 按行取块解码 + char_map（字节绝对偏移→(行,行内 cu16)，CodeMirror 坐标）；§5.2 行号与编码无关；§4.1 超 size_limit 拒开；Review Focus 1/5（split/对齐换算正确性）。
- 实现决策：
  - char_map v1 为“每逻辑行存 raw 跨度+解码文本，查询时逐字符走编码步进”（未做 ASCII 抽样压缩，已在 ledger 记 Ruling 缓至优化）；src_len：UTF-8 用 len_utf8、UTF-16 用 len_utf16*2、GBK/GB18030/BIG5/Shift-JIS 用逐字符再编码字节长（往返成立时精确）。
  - **修正 plan 假设**：plan Task4 Step1 设想“合并后 U+FFFD 消失”，但经实测 UTF-8/GBK/GB18030/BIG5/Shift-JIS 中 `0x0A` 从不是后续字节，合法流不可能被裸扫描误切；合并重试仅对畸形数据防御（测试改为“合法多行精确解码 + 畸形悬挂前导字节不 panic”）。
  - `finish` 契约（Task3 修订）在此消费：区域解码每块新建 IncrementalDecoder 从字节 0 语义→块内用 decode_all（BOM 仅文件首块）。
- 验证：`cargo test -p lumelog-core` → 26 lib + chunk_proptest(500) + docfile 5 + encoding_proptest 全 passed；`cargo clippy --all-targets -D warnings` 无告警。

### Task 4 修订（首轮 CodeReview 发现的缺陷）

- C-1 `chunk_at(_, 0)` 下流 panic（`line_start+count-1` 当 count=0）→ count==0 提前返回空 Chunk。
- I-1 `read_at` 短读被 `truncate` 静默吞掉→`build_chunk` 切片越界 panic（日志轮转/外部截断为主场景）→ `chunk_at` 校验 `bytes.len()==want`，不符返回 `CoreError::Io(UnexpectedEof)`；`to_rel` 加 `min` 二次防御。
- I-2 `decode_all` 对任意以 BOM 开头的切片都剥 BOM，与 `bom_skip` 仅 `ls==0` 不一致 → 非首块中部 `EF BB BF` 被误剥、文本丢一字符且 char_map 错位 → 新增 `encoding::decode_region`（从不剥 BOM），build_chunk 按 `ls==0` 选 decode_all/否则 decode_region。
- M-1 GB18030 合并重试循环对当前 9 种编码**不可达**（`0x0A` 从不是尾字节）→ 改为注释标注“对未来编码的防御”，测试新增断言 `logical_line_count==line_count`（证明未合并）。
- M-3 部分块（`start_line>0`）零覆盖→ chunk_proptest 新增 (D) 随机 start/count 子块与整块逐字节一致（钉 abs_start/start_line 记账一致性；I-2 由 (A) 内部 BOM 语料 + docfile 集成测试独立钉住）。
- 保留：M-2 UTF-16 整文件建索引的内存峰值（缓至流式 build_aligned 优化，ledger 记录）。
- 验证：`cargo test -p lumelog-core` → 36 全 passed（26 lib + 8 docfile + 2 proptest）；`cargo clippy --all-targets -D warnings` 无告警。首轮 CodeReview（区间 ef3fb81..5267a8f）裁定 Changes requested，二次 scoped re-review（5267a8f..aa01a27）裁定 Approved。

### Task 4 二次评审 polish（表述与防御）

- W-1（诚实性）：(D) 是子块与整块自比，实际钉不住 I-2 → chunk_proptest 语料补一条“首部 BOM + 行首内部 `EF BB BF`”，使 (A)（chunk 文本==整缓冲 decode_all）成为真正 oracle；订正 (D) 注释不再声称钉 I-2。
- S-1：docfile `interior_bom_is_preserved_as_content` 加 char_map 断言（byte 5→(1,0)、byte 8→(1,1)、line_count==3），把内部 BOM 的行内 cu16 对齐也钉住。
- S-2：`build_chunk` 的 `to_rel` 加 `debug_assert!`（abs∈[abs_start, abs_start+len]），`.min` 降级为纯 release 兜底，避免违约被静默掩盖。
- S-3：`line_index.rs` 陈旧测试注释订正——`0x0A` 从不是 GB18030 尾字节（4 字节序列尾域为 0x30-0x39），测试重命名为 `scanner_splits_on_every_lf_regardless_of_bytes` 以反映其真实意图（内容无关的裸扫描）。

## 2026-10-24 — Task 3 修订：CodeReview 发现的缺陷修复

- 文件：`src/encoding/mod.rs`、`src/doc/line_index.rs`、`tests/encoding_proptest.rs`。
- 为什么：首轮 CodeReview 实测暴露 3 Critical + 3 Important + 3 Minor，全部修复（不 park）。
  - Critical-1 `finish()` 丢弃 flush 文本 → `finish` 改为返回 `DecodeSegment`（带 text+invalid_count）。
  - Critical-2 flush 后复用 `encoding_rs::Decoder` 会 panic → `finish(mut self)` 消费所有权，杜绝复用。
  - Critical-3 BOM 跨小块、首块不足 bom_len 时漏剥 → 新增 `head` 缓冲累积到可判定再决定剥/BOM，`bom_done` 单次化。
  - Important：`invalid_count` 口径统一（decode_all 与逐段都数 U+FFFD，去掉 had_errors 门控）；detect 步骤2 容忍 64KB 窗口截断（`error_len().is_none()`）；proptest 空断言 → 语料补 UTF-16LE/BE+BOM、GBK/UTF-16 截尾、含字面 U+FFFD、随机字节，并消费新 finish 签名。
  - Minor：补 UTF-16BE 对齐扫描、builder >4GB 拒绝 TooLarge、UTF-16BE 往返+detect 测试。
- 验证：`cargo test -p lumelog-core` → 21 lib passed（含新确定性钉住用例 `bom_split_across_tiny_chunks_and_finish_flush`、`finish_returns_flushed_tail_text`，二轴覆盖 Critical-1/3）+ proptest(2000 cases) passed；`cargo clippy --all-targets -D warnings` 无告警。二次 CodeReview（区间 68168d9..8fa2178）裁定：6 项全修复、无新增 Critical/Important、Approved。

## 2026-10-24 — Task 3: 编码模块

- 文件：新增 `src/encoding/mod.rs`（EncodingLabel 9 种显示名 + as_str/from_label、detect、decode_all、IncrementalDecoder、encode_all）、`tests/encoding_proptest.rs`；lib.rs 挂 `pub mod encoding`。
- 为什么：实现 spec §5 检测顺序（BOM→严格 UTF-8→chardetng→回退 GB18030）与 §5.4 U+FFFD 错误策略；GB2312→GBK 超集（A3）；增量解码器为 Task4 分块取行服务，split-invariance 是正确性根基。
  - chardetng 0.1.17 无 float 置信度，用 `guess_assess(Some(b"cn"), true)` 的 plausible 标志实现 §5.2 的“置信阈值”（allow_utf8=true 让被截断的 UTF-8 头仍可落到 UTF-8）。
- 验证：`cargo test -p lumelog-core` → 16 lib passed + proptest split_anywhere passed；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 2: LineIndex 行索引

- 文件：新增 `src/error.rs`（CoreError：Io/TooLarge/LineOutOfRange/Encoding/Conflict/Unmappable/Locked）、`src/doc/line_index.rs`、`src/doc/mod.rs`；lib.rs 挂 `pub mod doc; pub mod error;`。
- 为什么：行边界按字节（仅 0x0A 分割），与编码无关 → 切编码保留行号（spec §5.2）；u32 偏移钉死 4GB 上限（§4.1/A9）；空文件=1行、CRLF 归上一行、UTF-16 偶数对齐扫描（Review Focus 3/5）。
- 验证：`cargo test -p lumelog-core --lib` → 8 passed（含 builder_chunks_equal_whole、utf16le_scan_even_aligned_only、gb18030 误切可索引）；`cargo clippy --all-targets -D warnings` 无告警。

## 2026-10-24 — Task 1: Workspace 脚手架

- 文件：新增根 `Cargo.toml`（workspace, members=lumelog-core/lumelog-can, workspace.dependencies 锁 memchr/encoding_rs/chardetng/regex/thiserror）；`crates/lumelog-core` 的 Cargo.toml/src/lib.rs/src/tests_common.rs/benches/core_bench.rs；`crates/lumelog-can` 空占位；`.gitignore`。
- 为什么：M0 起点，重逻辑放纯 Rust crate，不依赖 Tauri，可 CLI 复用与基准测试（spec 第3节）。core_bench 占位使 cargo test 不因缺失 bench 目标报错。
- 验证：`cargo test -p lumelog-core` → crate_links 1 passed。
