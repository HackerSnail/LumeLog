# Changelog

按 git log 时间倒序（最新在最上方）。每条写清：改了哪些文件、为什么、验证结果。

## 2026-10-31 — Plan 41 宏绑定快捷键（feat/macro-keys）

**依据：** `docs/specs/2026-10-31-macro-keys-design.md`（同类盘点第一梯队之二）。纯前端，Rust 零改动。

### Task. `macrokeys.ts` + 快捷键弹窗宏分区 + 键盘分发
**Files:** Create `app/frontend/src/macrokeys.ts`、`macrokeys.test.ts`（4）；Modify `app/frontend/src/keymap-ui.ts`（`KeymapModalOpts` 加 `macros/onMacroSet`；动作行后渲染「宏」分区（录制/清除，无默认键）；`recording` 放宽为 string（宏行 `macro:<名>` 前缀），录制查重跨动作/宏两张表——首版 `actionOwner` 查找漏排除自身已修，`apply(self)` 需 `as ActionId` 收窄）；Modify `app/frontend/src/main.ts`（`macroBindings` 存 `localStorage["lumelog.keymap.macros.v1"]`（宏名 → 规范化 accel，复用 keymap 的 normalize/validate/F3·Ctrl+F 保留规则）；`registerShortcuts` 在动作查空后 `macroForAccel` 命中即 `playMacro(1, ops)`；已保存宏以 `宏：<名>` 进命令面板、hint 显示绑定键；删宏连带删绑定；`openKeymapSettings` 传宏表与持久化回调）。
**Why:** 绑定不进 config.toml `[keymap]`——Rust 侧按 13 个动作 id 校验会丢弃动态宏名（unknown），且宏本体在 localStorage，绑定随宏走；查重必须跨动作/宏两张表，否则同一键两处抢。
**验证:** `npx vitest run src/macrokeys.test.ts` 4 pass（坏 accel/保留键/重名 accel 首到先得/空名丢弃/往返；conflictMacro 排除自身；macroForAccel 映射）；`npx tsc --noEmit` 净；`npm test` 212 pass（28 文件，基线 207）+ `npm run build` 通过。
**未验:** 录制绑定→按键触发宏需 `npm run tauri dev` 人工过。

## 2026-10-31 — Plan 40 工作区会话恢复（feat/session-restore）

**依据：** `docs/specs/2026-10-31-session-restore-design.md`（同类盘点第一梯队之首；后端 `doc_list` 是运行态查询，跨重启归前端存储）。纯前端，Rust 零改动。

### Task. `session.ts` 纯函数 + 开/关/激活三处快照 + 启动恢复
**Files:** Create `app/frontend/src/session.ts`、`session.test.ts`（4）；Modify `app/frontend/src/main.ts`（`saveSession()`：`tabs.list()` 开序 + `tabs.active()` 快照整表落 `localStorage["lumelog.session.v1"]`（normalize 去重保序/过滤空串/上限 50/active 不在集合置 null），挂在打开成功、handleClose 尾部、tabs onActivate 三处——每次开/关/切都写，崩溃也不丢；`restoreSession()` 启动序列末尾逐个 `openPath(path, {quiet: true})`，缺失/损坏文件静默跳过，完成后 toast「已恢复 N 个文档（M 个已缺失）」并激活记录页；`openPath` 加 `opts.quiet`）。
**Why:** LumeLog 无 untitled 缓冲，路径即会话；未保存改动由崩溃恢复影子另行负责，两者互不越界。恢复用 quiet 跳过错误弹窗——启动时连弹 N 个"文件不存在"不可接受。
**验证:** `npx vitest run src/session.test.ts` 4 pass（去重保序/上限/active 校验、坏数据、往返）；`npx tsc --noEmit` 净；`npm test` 207 pass（27 文件，基线 201）+ `npm run build` 通过。
**未验:** 开几个文档→重启→标签恢复且激活页正确需 `npm run tauri dev` 人工过。

## 2026-10-31 — Plan 39 IME 误提交守卫（跨平台适配，fix/ime-guard）

**依据：** test-checklist §4 最后一项 deferred（WKWebView/Safari 需叠 `keyCode!==229`）。纯前端，Rust 零改动。

### Task. `isIMEEvent` helper + 全部 Enter 提交点/快捷键录制器接管
**Files:** Modify `app/frontend/src/shell.ts`（`isIMEEvent`：`isComposing === true || keyCode === 229`——Safari/WKWebView 在 `compositionend` 之后才发提交用 Enter，`isComposing` 已是 false，229 是唯一可靠信号；结构化参数保持 headless 可测）；Modify `app/frontend/src/main.ts`（5 处 Enter 路径换守卫：标签输入、PDF 搜索、跳转行、转到行 Ctrl+G、palette 回车）；Modify `app/frontend/src/keymap-ui.ts`（快捷键录制 capture 阶段整段跳过 IME 事件——中文输入法开着录制会把 229/Process 录成快捷键或误关弹窗）；Modify `app/frontend/src/shell.test.ts`（+1 describe：两信号分别命中、普通键放行）。
**Why:** 逐点自查时发现守卫不一致：5 处 Enter 只有 `isComposing`（Safari 上不够），PDF 搜索完全没守卫，快捷键录制器最险（capture 阶段无差别吞事件）。统一走一个 helper 而不是各处复制条件，后续新输入框只引一个名字。
**验证:** `npx vitest run src/shell.test.ts` 7 pass；`npx tsc --noEmit` 净；`npm test` 203 pass（26 文件，基线 201）+ `npm run build` 通过。
**未验:** 真机 WKWebView/Safari 的中文输入法合成链路需 macOS 打包后实测（test-checklist §4 注明）。

## 2026-10-31 — Plan 38 书签跨会话持久化（feat/bookmarks-session）

**依据：** `docs/specs/2026-10-31-bookmarks-session-design.md`（Plan 28 spec §3 遗留项）。纯前端，Rust 零改动。

### Task. `bookmarks-store.ts` 纯函数 + handle 行号接口 + mountDoc 接线
**Files:** Create `app/frontend/src/bookmarks-store.ts`、`bookmarks-store.test.ts`（4）；Modify `app/frontend/src/editor.ts`（handle 加 `bookmarkLines()`（锚点 → 1-based 行号升序）与 `loadBookmarkLines(lines)`（越界丢弃、行号 → 行首锚点经既有 `setBookmarks` effect 批量装载）；`OpenEditableOpts.onBookmarksChanged(lines)` 在 toggle/clear 后回调，跳转不回调）；Modify `app/frontend/src/main.ts`（`localStorage["lumelog.bookmarks.v1"]` 读写；mountDoc 打开后 `loadBookmarkLines(已存行号)`，变更时整表替换该路径条目、空表删键）。
**Why:** 跨会话存**行号**而非字符偏移——磁盘上一次编辑就让偏移失效，行号才是稳定标识（N++ 同款语义）；文件变短后越界行号装载时丢弃。整表替换而非增量 diff，回调只带当前集合，逻辑最短；每次变化即落盘，崩溃也不丢。
**验证:** `npx vitest run src/bookmarks-store.test.ts src/editor.test.ts` 17 pass（parse 坏 JSON/去重排序/非正数与浮点丢弃/上限 200/往返；loadBookmarkLines 越界钳制 + 锚点就位 + bookmarkLines 回读；toggle/clear 触发 onBookmarksChanged 行号正确）；`npx tsc --noEmit` 净；`npm test` 201 pass（26 文件，基线 196）+ `npm run build` 通过。
**未验:** 打书签→关文档→重开仍在的跨会话链路需 `npm run tauri dev` 人工过。

## 2026-10-31 — Plan 37 后台流式索引：threaded doc_open + 8 MiB tick（feat/bg-index）

**依据：** `docs/specs/2026-10-31-background-index-design.md`（M1 遗留欠账，test-checklist §4）。core/shell/前端三层改动。

### Task. `open_deferred` + `index_step` + 后台线程 + 占位推进
**Files:** Modify `crates/lumelog-core/src/doc/line_index.rs`（`extend_u16`：偶对齐增量扫描 + 1 字节跨块 carry，奇数块边界不丢码元；`set_file_size`、`empty()`）；Modify `crates/lumelog-core/src/doc/file.rs`（`open` 重构为 `open_deferred` + step 循环，CLI/reload 语义不变；`index_step(chunk)` 按块推进 `indexed_bytes/index_done`，UTF-16 走 extend_u16；`try_append` 半截索引直接 NoChange；新 `IndexStep` 与 `indexed_bytes()/index_done()`）；Modify `app/src-tauri/src/commands.rs`、`dto.rs`（`doc_open` readonly 分支 `open_deferred` 后 spawn `doc-index-N` 线程逐 8MiB 块索引，每步短暂持锁 + Channel 发 tick（含 `indexed_lines` 新字段），关闭后的文档把余量索完（zombie 有界、tick 被前端忽略）；edit 模式原地循环索完（≤0.12s）语义同 v1；`total_lines: null` + `line_count_estimate = size/32`；reload tick 补字段）；Modify `app/frontend/src/ipc.ts`（`IndexProgress.indexedLines`）；Modify `app/frontend/src/main.ts`（`onIndexProgress` 重写：非 done tick 推进 `indexed` 占位分界并 120ms 节流刷新活动列表，done tick 校正 `ds.totalLines` + 状态栏 + 检查器；mountList 对延迟打开 indexed 初值 0）。
**Why:** 打开 1GB 只读日志原本要等全程建索引（P3 654ms）才首屏；占位行/refresh/Channel 前端在 Task 11 就位，只差 shell 线程化。UTF-16 增量扫描必须处理块边界劈开码元——1 字节 carry 让奇数块与全量 build_aligned 逐字节一致（测试钉死）。后台线程不设取消标记：关闭后的文档把剩余索完（有界），tick 被前端忽略，省一套生命周期 plumbing。
**验证:** `cargo test -p lumelog-core -- deferred extend_u16` 6 pass（extend_u16 奇数块流式 == build_aligned 全量、carry 跨块、deferred+step 与同步 open 行数/chunk_at 抽样一致、部分索引越界行 LineOutOfRange、半截索引 try_append NoChange、UTF-16 奇尾字节、空文件即 done）；`cargo test --workspace` 全绿 + `clippy -D warnings` 净；`npx tsc --noEmit` 净；`npm test` 196 pass + `npm run build` 通过。
**未验:** 1GB 日志打开即首屏、占位行随滚动推进的手感需 `npm run tauri dev` 人工过；部分索引下 filter_run 只覆盖已索引行（用户通常在索引完成后操作，未加阻断）。

## 2026-10-31 — Plan 36 发布物验证：Windows 打包 + P6 体积实测（chore/bundle-verify）

**依据：** `docs/test-checklist.md` §3 剩余可本地验证项。零代码改动（只修 CI 工作流一处调用）。

### Task. 本地 tauri build + P6 实测 + release.yml 修正
**Files:** Modify `.github/workflows/release.yml`；Modify `docs/test-checklist.md`（§1 P6 填数、§3 勾两项）。
**Why:** 打包验证时发现两处坑：① bundles 目录里躺着的其实是 10/24 M1 的老产物（时间戳核对才暴露，`npm run tauri build` 从 frontend 跑实际 panic 但 PowerShell 管道把退出码吞了——tauri-cli 的配置探测只看 cwd 与子目录，`src-tauri` 是 frontend 的兄弟目录不可见）；② release.yml 同款写法在 CI 也会踩。修法：从仓库根调 `app/frontend/node_modules/.bin/tauri`（配置向下可探测，beforeBuildCommand 仍解析到 frontend 的 package.json）。
**验证:** `tauri build`（仓库根）产出 NSIS `LumeLog_0.1.0_x64-setup.exe` **5.66 MiB**、MSI **8.95 MiB**（16:17 新鲜产物；裸 exe 21,252,608 B，含 wasmi+git2），P6 门槛 ≤15MB 达标约 2.6 倍余量，数字已填 test-checklist §1；前端 vite build 的 >500KB chunk 告警为既有现象（pdfjs chunk）。CI 三平台矩阵与 Gitee 发布上传须在 Actions 实跑，仍留 §3 未勾。
**未验:** macOS/Linux bundle、Actions 实跑、Gitee 发布（须 GITEE_TOKEN 与人工授权）。

## 2026-10-31 — Plan 35 Git 内置化（git2）+ 暂存并提交（feat/git-commit）

**依据：** `docs/specs/2026-10-31-git-commit-design.md`（用户点名：做 add/commit 且要内置 git）。`gitinfo.rs` 重写，命令层/前端扩展。

### Task. `gitinfo.rs` 换 git2 + `git_commit` 命令 + 提交对话框
**Files:** Modify `app/src-tauri/Cargo.toml`（`git2 = "0.20"`，libgit2 编译期内置）；Rewrite `app/src-tauri/src/gitinfo.rs`（探测走 `Repository::discover`；分支：head() shorthand，空仓库经 HEAD 符号目标抠 `refs/heads/<名>`，分离 HEAD 特判；文件状态 `status_file` 标志位 → 纯函数 `status_label`（暂存列优先，叠加未暂存改动时缀「另有…」）；最近提交 revwalk 逐提交比对该路径在父树中的 oid；`identity` 带出仓库/全局配置的 user.name/email；`git_stage_and_commit`：index add 当前文件 → write_tree → HEAD 为父（空仓库无父）`commit(Some("HEAD"),…)`，身份解析 纯函数 `resolve_identity`（显式参数 → 配置 → 报「缺少提交者身份」；姓名邮箱只填一半整体回退配置）；CLI 版 porcelain 解码与 run_git 全部删除）；Modify `app/src-tauri/src/commands.rs`（`git_commit` 命令，git_init 返回值适配）、`lib.rs` 注册；Modify `app/frontend/src/ipc.ts`（`identity` 字段 + `gitCommit` 封装）；Modify `app/frontend/src/main.ts`（file_status 非空时 Git 小节给「暂存并提交…」按钮；`openCommitDialog` 遮罩对话框：信息 textarea 必填 + 姓名/邮箱预填 identity，成功 toast 短 hash 并刷新小节）；Modify `app/frontend/src/styles.css`（`.commit-box/.commit-msg/.commit-field/.commit-actions`）。
**Why:** 内置 git 按用户要求换 libgit2——不依赖 PATH、错误结构化（不再解析 CLI 文本）；身份解析抽纯函数是因为集成测试会被开发机全局 git 配置污染，显式/配置/缺省三档只能单测。
**验证:** `cargo test -p lumelog-app gitinfo::` 3 pass（status_label 各组合；tempdir 集成 init→探测未跟踪→显式身份提交→状态转干净 + last_commit 命中新提交；resolve_identity 三档）；`cargo test --workspace` 全绿（14 个 result 全 ok）+ `clippy --workspace --all-targets -D warnings` 净；`npx tsc --noEmit` 净；`npm test` 196 pass + `npm run build` 通过。体积账：release `lumelog-app.exe` 18,904,576 → 20,553,728（git2 约 +1.6MB）。
**未验:** 真实仓库提交后与外部 git log/status 核对、对话框观感需 `npm run tauri dev` 人工过；libgit2 不跑钩子、不支持 GPG 签名（spec §3）。

## 2026-10-31 — Plan 34 检查器 Git 小节：仓库探测 / 初始化（feat/inspector-git）

**依据：** `docs/specs/2026-10-31-git-inspector-design.md`（NoteLume 检查器 Git 块对齐，用户在 Plan 33 后点名）。新增 Rust 模块，前端接线。

### Task. `gitinfo.rs` 探测/初始化 + 命令 + 检查器小节
**Files:** Create `app/src-tauri/src/gitinfo.rs`（`run_git` 参数数组不过 shell；`porcelain_file_status` 纯函数把 `status --porcelain` 首行 XY 映射中文（??/已忽略/已暂存×4/未暂存修改删除/其余原样）；`git_probe`：rev-parse → symbolic-ref → status → log -1 文件级，"not a git repository" 归为无仓库、其余失败归 `unavailable`；`git_init`）；Modify `app/src-tauri/src/commands.rs`、`lib.rs`（`git_probe/git_init` 两条命令注册，dir 取 `path.parent()`）；Modify `app/frontend/src/ipc.ts`（`GitInfo/GitLastCommit` 类型 + 封装）；Modify `app/frontend/src/main.ts`（updateInspector 追加 Git 小节壳，`fillGitSection` 异步探测回填——token + `dataset.path` 双重防旧结果覆盖；无仓库给「初始化仓库」按钮，ask 确认后 `gitInit` 并重探）；Modify `app/frontend/src/styles.css`（`.insp-git-init`）。
**Why:** 走 git CLI 而非 git2——零新依赖、不碰轻量定位；只读 + init 与参考项目对齐，add/commit 属版本管理工作流，塞进检查器反而越界；探测在激活/保存/重载时 on-demand 触发，不进热路径，无超时的风险可接受（spec §3 记录）。
**验证:** `cargo test -p lumelog-app gitinfo::` 3 pass（porcelain 各分支；tempdir 集成：无仓库探测 → init → 重探出分支/未跟踪 → add+commit 后文件状态转未改动、last_commit 出 hash/主题——无 git CLI 的环境自动跳过集成腿）；`cargo test --workspace` 全绿 + `cargo clippy --workspace --all-targets -- -D warnings` 净（首版 `len() > 0` 违规已改 `!is_empty()`）；`npx tsc --noEmit` 净；`npm test` 196 pass + `npm run build` 通过。
**未验:** 真实仓库下激活/保存后小节刷新、init 后按钮转信息行的观感需 `npm run tauri dev` 人工过。

## 2026-10-31 — Plan 33 检查器：文档属性分区 + 文档标签（feat/inspector-tags）

**依据：** `docs/specs/2026-10-31-inspector-tags-design.md`（用户截图 NoteLume 检查器，红框标「文档属性」「标签」两块）。Rust 侧零改动。

### Task. `doctags.ts` 纯函数 + 检查器分区 + 标签块
**Files:** Create `app/frontend/src/doctags.ts`、`doctags.test.ts`（6）；Modify `app/frontend/src/main.ts`（信息栏拆「文档属性」「标签」两小节；属性补「大小」「修改时间」——`DocState` 加 `size/mtime`，打开与外部重载两个写入点落值，tail 增长不回刷为已知限制；标签块 chips + 输入框，回车 `addTag`、× `removeTag`，操作即 `persistTags()` 落 `localStorage["lumelog.doc-tags.v1"]`，重复/满额 toast）；Modify `app/frontend/src/styles.css`（`.insp-section/.insp-tags/.insp-tag(-x)/.insp-tag-input`）。
**Why:** 标签键用文件路径而非 docId——标注属于文件，跨会话与重开都在；大小/修改时间不进 `TabEntry`（只有 docId/path），从打开时刻的 `DocInfo` 落进 `DocState` 最省事，重载点已有 info 可顺手刷新。
**验证:** `npx vitest run src/doctags.test.ts` 6 pass（normalize 截断、add 去重/满额/非法、remove 清空键、parse 坏 JSON/坏条目/上限钳制、序列化往返）；`npx tsc --noEmit` 净；`npm test` 196 pass（25 文件，基线 190）+ `npm run build` 通过。
**未验:** 标签增删后重启仍在（跨会话链路）、分区观感需 `npm run tauri dev` 人工过。

## 2026-10-31 — Plan 32 宏补遗：方向键/选区录制修复 + 跳词动作（feat/macro-selection-words）

**依据：** `docs/specs/2026-10-31-macro-selection-words-design.md`（收 Plan 30 留下的三块限制）。Rust 侧零改动。

### Task. `editortools.ts` 动作模型扩展 + `editor.ts` 捕获/键位拦截/重放
**Files:** Modify `app/frontend/src/editortools.ts`（`MoveBy` 加 `word-left/word-right`；`MacroOp.move` 加 `select?`；新增 `{kind:"del-word",dir}`；isMacroOp 校验同步）；Modify `app/frontend/src/editor.ts`（**修录制 bug**：核对 CM 源码发现光标命令的 userEvent 是 `select`，Plan 29 spec 写的 `input.move` 只在自家测试合成事务里出现——真实方向键从未进过宏；监听器改为 `input.move` 与 `select` 双路由，`select` 按锚点不变 + `classifyMove` 单步判据录 `{move,select:true}`，点击（锚点变）/跳页不录；keymap 在 defaultKeymap 之前加同键位绑定拦 Ctrl+←→（Mod/Alt 两套、含 mac 映射与 shift 处理）与 Ctrl+Backspace/Delete——CM 的 `deleteGroupBackward` 与 Backspace 事件同名，无法从事务区分，只能键位拦截；拦截期间 `suppressCapture` 标志防同一 dispatch 被 select/delete 捕获重复记录；重放：del-word 走 deleteGroup 系、word 走 cursorGroup 系、select 移动走 select 系命令、行首行尾 dispatch 保留锚点；重放 delete 时选区非空则删整个选区，"选中一段再输入"（录制为 delete+insert 对）得以还原；录制期事务涉及多选区直接跳过）；多光标明确不做，理由记录在 spec §3（动作流按主光标建模，忠实重放需逐光标重放再合并，与变更映射/撤销强耦合）。
**验证:** `npx vitest run src/editortools.test.ts src/editor.test.ts` 31 pass（新增：jsdom keydown 走键位拦截录 del-word 并重放出 "hello "；shift 选区 ×2 + 替换输入的录制形状与重放全文 "Xllo world"；序列化往返覆盖 word/select/del-word）；既有 29 个不回归（Plan 30 测试的合成 `input.move` 事务走 captureMove 路径不变）；`npx tsc --noEmit` 净；`npm test` 190 pass（24 文件，基线 188）+ `npm run build` 通过。
**未验:** 实机方向键/选区/跳词录制手感需 `npm run tauri dev` 人工过。

## 2026-10-31 — Plan 31 宏播放变体：重复 N 次 / 运行到文件尾（feat/macro-run-variants）

**依据：** `docs/specs/2026-10-31-macro-run-variants-design.md`（Plan 30 续作，补齐 N++ 的两个播放入口）。Rust 侧零改动。

### Task. `editortools.ts` 次数解析 + `editor.ts` playMacroToEnd + 命令面板
**Files:** Modify `app/frontend/src/editortools.ts`（`parseRepeatCount` 纯函数：纯数字且 ≥1 才收、钳到 `MACRO_PLAY_MAX` = 1 万，其余 null）；Modify `app/frontend/src/editor.ts`（重放循环从 handle 字面量抽成模块级 `replayOps(view, ops, times)`——`playMacroToEnd` 不能引用尚不存在的 `handle`，首版 `handle.playMacro(1)` 直接 TS2304；`playMacroToEnd()` 连续 `replayOps(recorded, 1)`，每次后比较文档长度与光标，两者都不变即停，返回实际播放次数，上限 1 万次兜底插入型宏）；Modify `app/frontend/src/main.ts`（命令面板加「宏：重复播放 N 次…」（onFreeText 输入次数，非法 toast）与「宏：运行到文件尾（无进展即停）」（播放完 toast 次数，无宏 toast））。
**Why:** "运行到文件尾"以文档长度 + 光标为无进展判据：下移、插行这类宏能按预期推进到尾部；排序/大小写不改长度且光标不动，播一次即停，与 N++ 以光标为准的行为相近，spec §3 已记录。
**验证:** `npx vitest run src/editortools.test.ts src/editor.test.ts` 29 pass（parseRepeatCount 合法/非法/钳制；下移宏运行到文件尾在最后一行停住、返回真实次数 2、空宏返回 0）；`npx tsc --noEmit` 净；`npm test` 188 pass（24 文件，基线 185）+ `npm run build` 通过。
**未验:** 实机播放手感与次数输入观感需 `npm run tauri dev` 人工过。

## 2026-10-31 — Plan 30 宏高级形态：编辑动作抽象 + 跨会话持久化（feat/macro-advanced）

**依据：** `docs/specs/2026-10-31-macro-advanced-design.md`（Plan 29 宏录制的续作）。Rust 侧零改动。

### Task. `editortools.ts` 动作模型 + `editor.ts` 捕获/重放 + 命令面板
**Files:** Modify `app/frontend/src/editortools.ts`（`MacroOp` 扩为四类：insert；delete 带 dir——backward 修 Plan 29 把 Backspace 重放成向后删的问题；move，`classifyMove` 纯函数按前后行列坐标归类，仅同行 ±1 列、相邻行、Home/End 可录，跳词/翻页/跨行折行不录；line，命名行操作；`serializeMacros`/`parseMacros` 逐条校验、坏宏丢弃，上限 50 个宏/名 100 字/每宏 1 万操作）；Modify `app/frontend/src/editor.ts`（updateListener 把 `input.move` 路由到新增的 `captureMove`——须放在 `u.docChanged` 分支外，纯移动不改文档；`handle.lineOp` 录制中记 `{kind:"line",op}`，重放共用抽出的 `applyLineOpToView`；`playMacro(times, ops?)` 支持显式 ops 与四类动作：backward 向光标前删、move 走 CodeMirror 命令或行首行尾 dispatch；handle 加 `macroOps()` 只读取已录制序列）；Modify `app/frontend/src/main.ts`（`localStorage["lumelog.macros.v1"]` 读写，启动加载；`openCommandList` 加 `onFreeText`——列表为空时回车提交输入文本，宏命名复用列表面板；命令面板加「宏：保存为…/运行已保存宏/删除已保存宏」三条，同名覆盖、删除即落盘）。
**Why:** 字节级宏重放完全依赖光标停在同样的相对位置，把移动与行命令做成命名动作后，"到行首再插入"这类序列才能跨内容复用；持久化选 localStorage 而非脚本那样的 `$APP_CONFIG` 文件，是因为宏体量小，且 UI 偏好已有同路。
**验证:** `npx vitest run src/editortools.test.ts src/editor.test.ts` 26 pass（classifyMove 各分支与不可录移动、序列化往返/坏数据/三项上限、extractMacroOps 方向；集成：录制 插入+移动+前删+行操作 → 重放全文一致、Backspace 重放向光标前删、`playMacro` 显式 ops、`macroOps()`）。首版两处错误已修：监听器的 move 捕获误放在 `u.docChanged` 分支内（纯移动永不触发）；测试里合成的插入事务没带 selection，光标没动导致移动不成立。`npx tsc --noEmit` 净；`npm test` 185 pass（24 文件，基线 175）+ `npm run build` 通过。
**未验:** 保存→重启→运行已保存宏的跨会话链路、箭头键实机录制手感，需 `npm run tauri dev` 人工过。

## 2026-10-30 — Plan 29 编辑器工具三件套：宏录制/概览条/函数列表（feat/editor-tools）

**依据：** `docs/specs/2026-10-30-editor-tools-design.md`（spec §5 缓议项开工）。Rust 侧零改动。

### Task. `editortools.ts` 纯函数 + `editor.ts` 接线 + 泛化列表面板
**Files:** Create `app/frontend/src/editortools.ts`、`editortools.test.ts`（8）；Modify `app/frontend/src/editor.ts`、`app/frontend/src/main.ts`、`styles.css`。
- **宏录制**：捕获用户输入事务（input.type/replace、delete.backward/forward），`iterChangedRanges` 折叠为 `{delete,count}|{insert,text}` 操作序列（上限 1 万）；播放从当前光标顺序重放（与 N++ 键盘宏同级的相对重放脆弱性，spec §1 明确记录）；handle `startRecording/stopRecording/isRecording/playMacro`（停止后宏保留可重复播放）；命令面板 3 条。
- **概览条（文档地图）**：编辑器右缘 36px canvas——`overviewBuckets` 分桶绘制非空行灰度、viewport 半透明窗框、点击/拖拽 `lineAtY` 跳行；`docChanged/viewportChanged/geometryChanged` rAF 合并重绘；行数 >50 万只画视口框（性能护栏）；开关默认开，面板切换。
- **函数列表**：`SYMBOL_PATTERNS` 按内置模式 id 的正则表（js/ts/python/rust/java/c/markdown/sql），`extractSymbols` 每行首命中；面板复用 openPalette 重构出的泛化 `openCommandList(placeholder, cmds, domId)`（符号即 Command：hint 行号、run revealLine）；无规则/只读 → toast。
**验证:** `npx vitest run src/editortools.test.ts` 8 pass（宏折叠与多 range 顺序、分桶边界/退化输入、y↔行双向映射、python/rust/markdown 符号提取、未知模式空表——修了 `lineAtY` 直传行号与 heading 取第二捕获组两处首版错误）；editor 集成 +2（录制→计数→重放插入"X"、符号形状）；`npx tsc --noEmit` 净；`npm test` 175 pass（24 文件，基线 165/23）+ `npm run build` 通过。openPalette 重构为泛化列表面板（行为不变，palette 测试回归）。
**未验:** 宏重放手感、概览条拖拽、函数列表观感需 `npm run tauri dev` 人工过。（附带清理：上一提交混入的临时文件 `npm-out2.txt` 已删除。）

**依据：** `docs/specs/2026-10-30-bookmarks-design.md`（Plan 27 spec §5 开工第一项；§5 盘点：列选择模式已随 `rectangularSelection()` 交付，宏录制/文档地图/函数列表继续缓议）。Rust 侧零改动。

### Task. `editorops.ts` 纯函数 + `editor.ts` StateField/gutter + palette
**Files:** Modify `app/frontend/src/editorops.ts`、`editorops.test.ts`（+2）；Modify `app/frontend/src/editor.ts`（`bookmarksPos` StateField：**排序字符偏移锚点**，`tr.changes.mapPos` 随编辑跟随；`bookmarkDeco` 行背景装饰（compute 派生）；独立 `cm-bookmark-gutter`（`lineMarker` 按锚点命中行返回 ● marker）；模块级 CM 命令 `toggleBookmarkCmd/nextBookmarkCmd/prevBookmarkCmd/clearBookmarksCmd` 与 keymap F2/Shift-F2/Ctrl-F2（与 app 级 keymap 无冲突）；handle 加 `toggleBookmark/nextBookmark/prevBookmark/clearBookmarks` 与只读 `bookmarkPositions(view)` 测试钩子）；Modify `app/frontend/src/main.ts`（命令面板加 4 条书签命令）；Modify `app/frontend/src/styles.css`（gutter 宽度/marker/书签行背景）。
**Why:** 锚点存字符偏移并用 changes 映射，增删行后书签自动跟行移动——存行号则每次编辑都要重算；纯函数拆出是为了环绕逻辑可脱离 CodeMirror 测试。
**验证:** `npx vitest run src/editor.test.ts src/editorops.test.ts` 20 pass（toggle 有序增删、next/prev 环绕、空表 null；经真实 EditorView 的 toggle→锚点断言→环绕跳转→清除）；`npx tsc --noEmit` 净（坑：`GutterMarker` 属 `@codemirror/view` 而非 search 包）；`npm test` 165 pass（23 文件，基线 162）+ `npm run build` 通过。
**限制（spec §3）：** 书签为会话内状态，随文档关闭丢失；跨会话持久化需挂会话/恢复文件，后续按需。

**依据：** `docs/specs/2026-10-30-editor-parity-design.md`。盘点结论：查找/替换（`@codemirror/search` 面板）、转到行（Ctrl+G）、行复制/移动/删除（defaultKeymap）已在，缺口为行批处理、换行切换（原常开）、字号缩放、选区匹配高亮。Rust 侧零改动。

### Task. `editorops.ts` + `editor.ts` Compartments + palette 命令
**Files:** Create `app/frontend/src/editorops.ts`、`editorops.test.ts`（11）；Modify `app/frontend/src/editor.ts`（`OpenEditableOpts.wrap` 初值；`wrapConf/fontConf` 两个 Compartment（换行与字号，运行时切换不丢撤销/滚动）；`highlightSelectionMatches()` 选区匹配高亮；Ctrl+滚轮字号缩放（`domEventHandlers` 截获防浏览器页面缩放）；handle 加 `setWrap/isWrap/lineOp/zoomFont/fontReset`，`lineOp` 用单次 dispatch 保证一步撤销，空选区 = 整篇）；Create `app/frontend/src/editor.test.ts`（+2：整篇排序/反转/去重、换行开关）；Modify `app/frontend/src/main.ts`（`editorWrapOn` 持久化 `lumelog.ui.wrap` + `activeEditor()`；命令面板新增 13 条：切换自动换行、字体 ±/重置、行排序升/降、去重、反转、大小写、Trim、去空行、合并为一行；`openEditable` 传 wrap 初值）。
**Why:** 行操作做成纯函数 + 单 dispatch 是为了一步撤销与可测性；换行原是常开静态扩展，与 N++ 的可切换行为不符；字号缩放不持久化与 N++ 会话级行为一致。
**验证:** `npx vitest run src/editorops.test.ts src/editor.test.ts` 15 pass（9 种行操作含不变形、字号钳制 9–28、整篇排序/反转/去重经真实 EditorView、换行开关三态）；jsdom 缺 `Range.getClientRects`（`highlightSelectionMatches` 依赖），测试里 `beforeAll` 打桩；`npx tsc --noEmit` 净；`npm test` 162 pass（23 文件，基线 148/22）+ `npm run build` 通过。
**未验:** Ctrl+滚轮缩放与选区行操作的实机手感需 `npm run tauri dev` 人工过。书签/列选择/宏录制不在本 plan（spec §5）。

### Task 2. 文档收尾
**Files:** Modify `docs/features.md`（新增对齐条目；插件接口条目从「待做」移入「已交付」、删过期提案注记；修复编辑器条目误顶脚本引擎标题的结构问题）。
**验证:** 纯文档改动，未动代码，不涉及测试。

**依据：** `docs/specs/2026-10-30-pdf-select-thumbs-design.md`（Plan 22/24 的配套增量）。Rust 侧零改动。

### Task. TextLayer 文本层 + 缩略图栏
**Files:** Modify `app/frontend/src/pdfview.ts`（纯函数 `thumbScale`（0.05–4 钳制）；`render()` 后叠 pdf.js `TextLayer`（CSS 像素 viewport，与设备像素 canvas 的 CSS 显示尺寸对齐；fire-and-forget，旧层在下次渲染时整体替换、`textLayer !== layer` 防旧层回写）；`initThumbs(rail, onGoPage)`：每页一个 cell，`IntersectionObserver`（rootMargin 200px）懒渲染 88px 宽缩略图，`setThumbCurrent` 边框跟随当前页；load/destroy 清 rail 与 observer）；Modify `app/frontend/src/main.ts`（dock 里 `.pdf-body` 横向容器包缩略图栏 + 渲染区，pdf 模式下 load 后 `initThumbs`、点击跳页，工具条加「缩略图」显隐切换，默认开）；Modify `app/frontend/src/styles.css`（`.textLayer` 最小样式——pdfjs 6 不自动注入：absolute、`color: transparent`、`::selection` 半透明 accent、`user-select: text`；`.pdf-body/.pdf-thumbs/.pdf-thumb(-current)`）。
**Why:** TextLayer 的 viewport 必须用 CSS 像素 scale——canvas 是设备像素尺寸再 CSS 缩放显示，文本层跟 CSS 尺寸对齐才能选得准；缩略图懒渲染避免大 PDF 一次性渲染上百页。
**验证:** `npx vitest run src/pdfview.test.ts` 6 pass（新增 `thumbScale` 钳制/退化页；既有测试不回归——构造后仍是 1 个 canvas、destroy 后 0）；`npx tsc --noEmit` 净（坑：pdfjs 6 的 `TextContent` 类型要求 `styles` 与可空 `lang`，类型从 `pdfjs-dist/types/src/display/api` 深路径导入）；`npm test` 148 pass（22 文件）+ `npm run build` 通过；Rust 零改动。
**未验:** 文本选择的对齐精度与缩略图懒加载观感需 `npm run tauri dev` 人工过（jsdom 无排版）。

**依据：** `docs/specs/2026-10-30-pptx-text-design.md`。PPT 真渲染（版式/主题/图片）明确不做，本增量以文本提取收口 features.md 的 Office 条目；.ppt 老二进制格式不支持。

### Task. `viewers.rs` 接入 pptx
**Files:** Modify `app/src-tauri/src/viewers.rs`（`ViewerKind::Pptx`；`open_pptx()`：zip 收集 `ppt/slides/slide<N>.xml` **按 N 数值排序**（字典序会把 slide10 排到 slide2 前），每张幻灯片前插 `── 幻灯片 N ──` 分隔行，`<a:p>` 段落 → 行、`<a:t>` 跨 run 拼接；段落/文本上限沿用，超限截断标记；无 slides 条目 → `invalid_argument`）；新增共享解析器 `xml_paragraph_texts(xml, p_tag, t_tag)` 并把 docx 提取重构到其上（消除两份几乎相同的 quick-xml 循环；行为差异：docx XML 截断从报错改为容忍已解析部分——与 pptx 一致），上限统一走 `apply_text_caps`）；Modify `app/frontend/src/main.ts`（查看对话框 filter 加 `pptx`，导出走既有 TXT 分支）。
**验证:** `cargo test -p lumelog-app viewers::` 8 pass（新增：幻灯片数值排序 1/2/10、段落切分与分隔行、空幻灯片（`<a:p/>`）单空行、TXT 导出同源、无 slides 条目拒绝；既有 docx 测试在共享解析器重构后不回归）；`cargo test --workspace` 全绿 + `clippy -D warnings` 净；`npm test` 147 pass + `npm run build` 通过。
**未验:** 真实 pptx（图片/表格/备注页）只出文本，观感需 `npm run tauri dev` 人工过。

**依据：** `docs/specs/2026-10-30-pdf-search-design.md`（Plan 22 真渲染的配套增量）。Rust 侧零改动。

### Task. `pdfview.ts` 文本层搜索 + 命中高亮 + dock 搜索 UI
**Files:** Modify `app/frontend/src/pdfview.ts`（纯函数 `matchItem`（item 内大小写不敏感命中起点，含重叠命中）与 `hitRect`（内联 pdf.js `Util.transform` 矩阵乘法，命中框 = item 宽度按字符比例横向切分，设备坐标 ÷ dpr 得 CSS 像素）；`PdfView` 加每页 `getTextContent()` 缓存、`searchAll(query, onProgress)`（逐页全文替换式搜索）、`gotoHit(index)`（切页渲染 + 滚动定位）、`clearSearch()`、`.pdf-page-wrap > .pdf-hits` 覆盖层（渲染后按当前 viewport 重画命中框，缩放/翻页跟随，当前命中强调色）；load 时清 per-doc 缓存与命中）；Modify `app/frontend/src/main.ts`（pdf 工具条加查找输入 + 命中数 + ◀▶ + ✕：Enter 全文搜索跳第一条、同查询只循环命中、Esc 清除，换文档重置搜索状态）；Modify `app/frontend/src/styles.css`（`.pdf-page-wrap` 相对定位 + `.pdf-hit`/`.pdf-hit-current` 命中框）；Modify `app/frontend/src/pdfview.test.ts`（+2：匹配含重叠命中/空查询、矩形比例切分与 dpr 换算——首版断言手算漏了 item x 偏移随缩放，已按矩阵乘法结果修正）。
**Why:** 不用 `PDFFindController`——它绑死 `PDFViewer` 全套组件会替换现有 canvas 渲染路径；`getTextContent()` + 几何换算代码量 1/5。跨 item 匹配与字形度量近似是明确记录的限制（spec §3）。
**验证:** `npx vitest run src/pdfview.test.ts` 5 pass；`npx tsc --noEmit` 净；`npm test` 147 pass（22 文件）+ `npm run build` 通过。
**未验:** 真实 PDF 的搜索观感（CJK item 切分、命中框近似偏差）需 `npm run tauri dev` 人工过。

**依据：** `docs/specs/2026-10-30-plugin-wasm-impl-design.md`（立项 `2026-10-30-plugin-wasm-proposal.md` 建议路线 B：wasmi 解释器）。插件接口三增量至此全部交付。

### Task. wasmi 宿主 + 列表/运行分派
**Files:** Create `app/src-tauri/src/wasmi_host.rs`（`WasmPlugin`：wasmi 2.0 实例化 + `StoreLimitsBuilder.memory_size(16MB)` + 每次调用重置 fuel 5×10⁸（单次死循环被截断）；朴素 wasm ABI——插件导出 `memory/alloc/meta/parse/finish`，i64 打包 `(ptr<<32)|len` 指向插件内存内的 UTF-8 JSON，`parse(chunk, base_line)` 喂 256KB 完整行块、行号约定 `base_line + 块内换行计数`，EOF 调 `finish` 冲刷尾记录；`run_wasm_parser`：`DocLines` 逐行累积 → `parse` → 512 行/批发帧，`RunCtx` 统一取消/deadline/错误处理）；Modify `app/src-tauri/src/plugins.rs`（`ParserPluginInfo/ParserColumnJson/ParserRowJson` 加 Deserialize；`list_parser_plugins`/`find_parser_plugin` 扩到 `*.js|*.wasm`，wasm 走 wasmi 实例化 + `meta()` 且元数据过 `validate_parser_info` 形状校验；id 去重/文件名序/单文件失败不阻塞沿用）；Modify `app/src-tauri/src/commands.rs`（`parser_run` 按扩展名分派 js/wasm）；Modify `app/src-tauri/Cargo.toml`（`wasmi 2.0`；dev-dep `wasm-encoder 0.262`）；`lib.rs` 注册。
**Why:** ABI 用朴素 wasm 而非 wit-bindgen——首版把 ABI 面压到最小，JSON 进出与前端 `ParserRow` 同构（表格/导出零改动），wit 冻结推迟到有第二实现时；wasmi（解释器）按立项比选，体积约为 wasmtime 的 1/10。
**验证:** `cargo test -p lumelog-app wasmi_host::` 2 pass——fixture 用 `wasm-encoder` 程序化构建 wasm 模块（无需 wasm 工具链；坑：函数体需显式 `End`、data 段偏移的 LEB128 手写 2048/4096）：常量行插件列表出数 + `parser_run` 出行（尾块 1 行 + finish 1 行）、空输出插件 lines 计数正确、坏 wasm 报错不阻塞其它插件；`cargo test --workspace` 全绿 + `clippy -D warnings` 净；`npm test` 142 + `npm run build` 通过（前端零改动）。体积核对：release `lumelog-app.exe` 基线 16,732,160 → 含 wasmi 18,904,576（+2.07MB，符合立项预估 2–3MB）。
**未验:** 真实 Rust 编译的 wasm 插件（cargo 模板见 `docs/specs/2026-10-30-plugin-wasm-impl-design.md` §3）走通"编译→丢进 plugins→表格出数"需人工过。

**依据：** `docs/specs/2026-10-30-pdf-render-design.md`；立项文档比选后选路线 C（pdf.js，无原生库分发成本）。增量1 的文本提取保留为 TXT 导出/`rows()` 按需能力。

### Task 1. Rust 会话改造 + 字节命令
**Files:** Modify `app/src-tauri/src/viewers.rs`（会话加 `pdf_path`；`open_pdf` 不再在打开时提取——提取失败的 PDF 仍可渲染；`ensure_pdf_lines()`：`rows()`/`export()` 按需提取（catch_unwind 兜 panic 面）；`ViewerSession/get()` pub(crate) 供命令层）；Modify `app/src-tauri/src/commands.rs`（`viewer_pdf_bytes`：返回 `tauri::ipc::Response` 原始字节——JSON 数组会让 64MB 膨胀 33%）；`lib.rs` 注册。
**验证:** `cargo test -p lumelog-app viewers::` 6 pass（新语义：畸形 PDF **open 成功**、`rows()`/`export()` 才报 `invalid_argument`；好 PDF 按需提取内容与 TXT 导出回归）；`cargo test --workspace` 全绿 + `clippy -D warnings` 净。字节命令是薄层（会话路径 → fs read → Response），路径与读取由会话测试覆盖。

### Task 2. `pdfview.ts` + 前端桥
**Files:** Modify `app/frontend/package.json`（`pdfjs-dist@6`）；Create `app/frontend/src/pdfview.ts`、`pdfview.test.ts`（3）；Modify `app/frontend/src/ipc.ts`（`viewerPdfBytes(id): Promise<ArrayBuffer>`，错误翻译与 `call` 同路）。
`PdfView`：动态 `import("pdfjs-dist")`（主包零增长）、worker 经 `?url` 静态 import 随本模块进 pdf chunk、`getDocument({data})` 直收后端字节；`render(page, scale)` 用 DPR 缩放 canvas，旧 RenderTask `cancel()` 防翻页竞态；纯函数 `clampPage/stepZoom`（×1.25，0.5–4）单独导出可测。文档销毁用 `cleanup()`（pdfjs-dist 6 的类型里 PDFDocumentProxy 无 destroy）。
**验证:** `npx tsc --noEmit` 净；`npx vitest run src/pdfview.test.ts` 3 pass（页码钳制边界、缩放上下限、未 load/destroy 后 render 为 no-op 且零错误）。

### Task 3. dock 两态切换（`main.ts` + 样式）
**Files:** Modify `app/frontend/src/main.ts`（`buildViewerDock` 加 pdf 工具条（◀/页码输入/共 N 页/▶、缩放下拉 0.5–4 + ＋/－（`stepZoom`）+ 适应宽度）与 `.pdf-host` 宿主；`setViewerMode("pdf"|"table")` 两态互切（表格控件隐藏、导出按钮文案 CSV↔TXT）；`openViewerFile` kind==="pdf" 时 `viewerPdfBytes` → `pdfView.load` → 首页渲染；`renderViewerPdf` 同步页码/标签并记录 `viewerPdfBaseWidth`（适应宽度的基准）；dock 收起/切回表格即 destroy）；Modify `app/frontend/src/styles.css`（`.pdf-host` 滚动容器 + `.pdf-canvas` 页面卡片阴影）。
**Why:** `render` 返回渲染后 CSS 宽度，`适应宽度` 才能从任意缩放换算基准页宽；表格路径在 pdf 模式下整体让位，避免隐藏的虚拟表格继续响应滚动。
**验证:** `npx tsc --noEmit` 净；`npm test` 145 pass（22 文件）+ `npm run build` 通过；主 chunk 521.5KB（Plan 15 基线 506.9，增量来自 Plan 20/22 的 UI 代码），pdfjs 落独立异步 chunk（103.9KB）+ worker 按需资产（1.26MB），主包约束满足。另清掉误入上一提交的临时文件 `npm-out.txt`。
**未验:** 真实多页 PDF 的翻页/缩放观感、CJK 字体渲染需 `npm run tauri dev` 人工过。

## 2026-10-30 — Plan 21 PDF 查看增量1：纯 Rust 文本提取（feat/pdf-text）

**依据：** `docs/specs/2026-10-30-pdf-viewer-proposal.md` 建议的分步立项增量1（文本提取先行），延续 Plan 19 查看器体系。真渲染增量（pdf.js/pdfium）仍待需求反馈另立项。

### Task. `viewers.rs` 接入 PDF 文本提取
**Files:** Modify `app/src-tauri/src/viewers.rs`（`ViewerKind::Pdf`；`open_pdf()`：`pdf-extract 0.12` 提取全文 → 按行落文本行会话（与 docx 同一 `docx_lines` 路径，沿用段落/文本上限与截断标记），`catch_unwind` 兜畸形 PDF 的 panic 面 → `invalid_argument`；`rows()`/`export()` 分派改为按「文本行是否存在」而非 kind，pdf 与 docx 共用单列「内容」与 TXT 导出）；Modify `app/frontend/src/main.ts`（查看对话框 filter 加 `pdf`；导出按 `kind === "xlsx"` 分 CSV/TXT）。
**Why:** 提取文本无版面，落进 docx 同款"单列文本 + 行号"视图零新增 UI；扫描件（无文本层）会得到空行会话，不为它做 OCR。
**验证:** `cargo test -p lumelog-app viewers::` 6 pass（新增：手造最小单页 PDF——对象/xref/偏移量程序化拼装——提取两行文本、TXT 导出同源、畸形输入返回错误而非 panic）；`cargo test --workspace` 全绿 + `clippy --workspace --all-targets -- -D warnings` 净；`npm test` 142 pass + `npm run build` 通过。
**未验:** 真实 PDF（CJK 字体/扫描件）提取质量需 `npm run tauri dev` 人工过。

**Files:** Create `docs/specs/2026-10-30-pdf-viewer-proposal.md`、`docs/specs/2026-10-30-plugin-wasm-proposal.md`；Modify `docs/features.md`（两条待做挂立项文档链接）。
**Why:** 剩余两项此前只有一句推迟理由，缺可比选的立项材料。PDF 给出三路线比选（pdfium 真渲染 / 纯 Rust 文本提取 / 前端 pdf.js），建议拆两步：文本提取先行（复用 Plan 19 查看器，半天级），真渲染默认 pdf.js（无原生分发成本、体积 1/3）；Wasm 宿主给出四路线比选，建议 wasmi 起步（体积约 wasmtime 的 1/10，wit 接口与运行时解耦，预留切换），附 wit 心智稿、沙箱边界与体积/门槛风险。两份均为预研，未批准实现；决策点（文本先行与否、路线选择、15MB 体积硬门槛）单列待拍板。
**验证:** 纯文档改动，未动代码，不涉及测试。

## 2026-10-30 — docs：test-checklist §4 拖放/最近文件欠账标记已还清

**Files:** Modify `docs/test-checklist.md`。
**Why:** Plan 20 已交付拖放打开与最近文件，deferred 清单对应条目标记完成，避免双份追踪。
**验证:** 纯文档措辞修改，未动代码，不涉及测试。

## 2026-10-30 — Plan 20 拖放打开 + 最近文件（feat/drop-recent）

**依据：** spec `docs/specs/2026-10-30-drop-recent-design.md`；计划 `docs/plans/2026-10-30-drop-recent.md`。偿还 test-checklist §4 记录的 M1 欠账（拖放打开、最近 20 文件菜单未接线）。

### Task 1. Rust 最近文件存储（`recent.rs` + `doc_open` 挂钩）
**Files:** Create `app/src-tauri/src/recent.rs`（`recent.json` 存 `["path",…]`，`record_recent` 去重置顶 + 截断 20 + tmp→rename 原子写；`load_recent` 缺失/损坏 JSON → 空表；`clear_recent`；命令 `recent_list`（读时过滤已不存在路径）/`recent_clear`）；Modify `app/src-tauri/src/commands.rs`（`doc_open` 成功后记录——对话框/拖放/palette 全部打开入口自动覆盖）；`lib.rs` 注册。
**Why:** 记录取后端挂钩而非前端逐入口上报，覆盖面完整且不依赖 UI 层记得调用；文件是 advisory 状态，任何读写失败都不影响打开主流程。
**验证:** `cargo test -p lumelog-app recent::` 4 pass（缺失/损坏 → 空、去重置顶 + 第 21 条挤最旧、clear 后可再记录且无 tmp 残留、落盘形态为纯字符串数组）；`cargo test --workspace` 全绿 + `clippy --workspace --all-targets -- -D warnings` 净。

### Task 2. 前端（拖放 + palette 最近条目）
**Files:** Modify `app/frontend/src/ipc.ts`（`recentList/recentClear`）、`ipc.test.ts`（+1 typeof）、`app/frontend/src/main.ts`（`initRecent/refreshRecent/clearRecentList`：启动读一次、`openPath` 成功后刷新；`initDragDrop()`：动态 `import("@tauri-apps/api/webview")` 的 `onDragDropEvent`，`enter/over` 显示全屏遮罩、`drop` 顺序 `openPath`（沿用 busy 单飞，首个失败停批）、非 Tauri 环境降级；`buildCommands()` 顶部注入 `recent-<i>` 条目（标题=文件名、hint=完整路径，最多 10 条）+「清除最近文件记录」——id 前缀 `recent-` 在 keymap 动作空间之外）；Modify `app/frontend/src/styles.css`（`.drop-overlay`：fixed 全屏、`pointer-events:none` 不抢拖拽）。
**Why:** Tauri v2 webview 拿不到 File 路径，必须走 webview 级 drag-drop 事件；属性窄化不进嵌套 async 闭包，drop 的 paths 先捕获再进闭包（tsc 实际报错后修正）。
**验证:** `npx tsc --noEmit` 净；`npm test` 142 pass（21 文件）+ `npm run build` 通过。
**未验:** 真实拖放与 palette 跳转需 `npm run tauri dev` 人工过（headless 无拖拽）。

## 2026-10-30 — Plan 19 文档查看器 增量1：xlsx 表格 + docx 文本（feat/doc-viewer）

**依据：** spec `docs/specs/2026-10-30-doc-viewer-design.md`；计划 `docs/plans/2026-10-30-doc-viewer.md`。对应 `docs/features.md` 待做第 2 条。范围裁剪：PDF（需 pdfium 级渲染引擎，体积/分发代价大）与 PPT（版式复杂、文本提取价值低）推迟；插件接口增量3（Wasm host）同步推迟——需引入 wasmtime 级依赖与"轻量"定位冲突，且 JS 沙箱插件已覆盖高亮与逐行解析。

### Task 1. Rust 查看器会话（`viewers.rs`）
**Files:** Create `app/src-tauri/src/viewers.rs`（`ViewerSessions`：id→会话上限 4；`viewer_open` 按扩展名分流——xlsx/xlsm 走 `calamine::Xlsx`（BufReader<File> 常驻），docx 用 zip + quick-xml 提取 `word/document.xml` 段落（跨 `w:r` 拼接 `w:t`、`<w:p/>` 空段计一行、单条目解压 ≤64MB、段落 ≤20 万/文本 ≤64MB 超限截断标记）；`rows()`：xlsx 物化 Range 后窗口跳取（limit ≤1000），窗口统一列宽（最远非空列、≤64 列），单元格字符串化（整数浮点去尾零、Bool→TRUE/FALSE、DateTime→`YYYY-MM-DD HH:MM:SS`、Error→`#<err>`），列字母表头；docx 单列「内容」；`export()`：xlsx 全表 CSV（UTF-8 BOM、无表头、RFC 引号转义、上限 1,048,576 行、tmp→rename）+ docx TXT）；Modify `app/src-tauri/src/commands.rs`（`viewer_open/viewer_rows/viewer_close/viewer_export_csv`；会话托管为 `Arc<Mutex<ViewerSessions>>`，导出 async + spawn_blocking 防大表卡 IPC 线程）；`lib.rs` 注册）；Modify `Cargo.toml`（`calamine 0.37`+chrono、`zip 9`(deflate)、`quick-xml 0.37`——0.42 移除了 `BytesText::unescape` 故回退；dev-dep `rust_xlsxwriter`）。
**验证:** `cargo test -p lumelog-app viewers::` 5 pass（xlsx：sheet 列表/窗口行/整数与日期格式化/统一列宽补空/越界拒绝；docx：跨 run 拼接与空段；未知扩展名 `viewer_unsupported`；会话上限 4 + close 幂等释放；CSV 与表格同源 + BOM/引号、TXT 导出；列字母 A/Z/AA/BL）。fixture 用 `rust_xlsxwriter` 现写（日期单元格必须带日期 num-format，calamine 才会读成 DateTime 而非裸序列号）+ `zip` 现写 docx。`cargo test --workspace` 全绿 + `clippy --workspace --all-targets -- -D warnings` 净。

### Task 2. 命令 + 前端桥
**Files:** Modify `app/frontend/src/ipc.ts`（`ViewerInfo/ViewerRow/ViewerPage/ViewerExportStats` + `viewerOpen/viewerRows/viewerClose/viewerExportCsv`；错误码 `viewer_unsupported` 进联合与中文表）；Modify `app/frontend/src/ipc.test.ts`（+2）。
**验证:** `npx tsc --noEmit` 净；`npx vitest run src/ipc.test.ts` 23 pass。

### Task 3. 前端 dock（`main.ts` + 样式）
**Files:** Modify `app/frontend/src/main.ts`（工具栏「查看」按钮 + 文件对话框；`buildViewerDock()`（sheet 下拉 + 「加载后续 1000 行」 + 导出）；`openViewerFile()/toggleViewerDock()`（四个底部 dock 互斥，收起即 close 会话）；`loadViewerRows()`：按窗口拉取 + `viewerToken` 丢弃旧响应 + 状态行 `文件 · N/总 行 · 已截断`；`exportViewer()` 按 kind 出 CSV/TXT save 对话框；复用 `ParserPanel/RowStore`，行号列不接跳转）；Modify `app/frontend/src/styles.css`（`#viewer-dock` 并入 dock 高度契约选择器）。
**Why:** xlsx/docx 的行数据 `ViewerRow{line,cells}` 与 `ParserRow` 同构，直接复用增量2 的虚拟表格，零新表格代码。
**验证:** `npx tsc --noEmit` 净；`npm test` 141 pass（21 文件）+ `npm run build` 通过。
**未验:** 真实 xlsx/docx 文件的查看与导出观感需 `npm run tauri dev` 人工过。

### Task 4. 文档收尾
**Files:** Modify `docs/features.md`（待做第 2 条注记：xlsx/docx 已交付，PDF/PPT 推迟及原因）。
**验证:** 纯文档改动，未动代码，不涉及测试。

## 2026-10-30 — Plan 18 插件接口 增量2：QuickJS 数据解析插件（feat/plugin-parser）

**依据：** spec `docs/specs/2026-10-30-plugin-parser-design.md`；计划 `docs/plans/2026-10-30-plugin-parser.md`。接 Plan 17，覆盖 `docs/features.md` 待做第 1 条的「冷门日志与报文格式」；Wasm host 为增量3。

### Task 1. 解析器元数据加载（`plugins.rs` + `parser_plugins_list`）
**Files:** Modify `app/src-tauri/src/plugins.rs`（共享校验助手 `check_id/check_name/check_extension` 抽出供两类插件复用；`ParserColumnJson/ParserPluginInfo/ParserPluginsReply`；`load_parser_metadata()`：rquickjs 沙箱执行 `*.js` 一次、读顶层 `plugin` 对象（`var` 走 global、`const/let` 走 script.rs 同款回退 eval）、校验 id/name/extensions/columns（key 1–32 `[A-Za-z0-9_]` 不重复、label 1–64、列 1–32）且 `parseLine` 必须为函数；`list_parser_plugins()`：文件名序、单文件失败不阻塞、id first wins、单文件 5s 超时 + 全表 15s 预算；命令 `parser_plugins_list`）；Modify `app/src-tauri/src/script.rs`（`MEMORY_LIMIT/STACK_LIMIT/CONSOLE_SHIM/catch_message` 改 `pub(crate)` 供复用）；`lib.rs` 注册。
**Why:** 元数据（id/列定义）必须执行才能拿到，但只执行一次且不调用 `parseLine`；列定义在前端建表格需要 Rust 侧先验形状。15s 预算防「一个死循环插件拖死整个列表」。
**验证:** `cargo test -p lumelog-app plugins::` 13 pass（新增：合法元数据、坏形状 4 例各自报错带文件名、顶层 `const plugin` 可读到、坏 JS 不阻塞其它、id 重复 first wins、`.json` 不混入解析器列表）；`cargo test --workspace` 全绿 + `clippy --workspace --all-targets -- -D warnings` 净。

### Task 2. 流式执行 + 命令（`run_parser_plugin` + `parser_run/parser_cancel`）
**Files:** Modify `app/src-tauri/src/plugins.rs`（`ParserRowJson{line,cells}`/`ParserStats{rows,lines,skipped,cancelled,error}`；`CELLS_SHIM`：行对象→列序字符串数组的转换放 JS（`String(number)` 保持 JS 语义），不支持的值类型返回 `null` → Rust 置空 + 一次性 console warning；单元格截断 4096 字符；`run_parser_plugin()`：独立沙箱（64MiB/1MiB 栈/300s + interrupt handler）、`DocLines` 逐行调 `parseLine(line, ctx)`、512 行/批 emit、`find_parser_plugin()` 按 id 后端定位文件；`read_columns()` 运行时复读列 key）；Modify `app/src-tauri/src/commands.rs`（`parser_run`：can_parse 式异步双通道 + `on_console` 第三通道、`begin_search/end_search` 取消注册、`spawn_blocking` + `catch_unwind` 兜底"解析运行器内部异常"；`parser_cancel`）。
**Why:** 前端只传插件 id 不传路径（防注入）；取消注册复用全局 cancel 表；值转换放 JS 避免 Rust 重写 JS 数字格式化。
**验证:** `cargo test -p lumelog-app plugins::` 17 pass（端到端：行→cells 顺序/number→"1"/对象列置空+一次性 warning、null 跳过计数、cancel 恰停 1 行、JS 抛错带行号、console.log 透传、按 id 定位/未知 id `plugin_not_found`）；`cargo test --workspace` 全绿 + `clippy -D warnings` 净。

### Task 3. 前端桥（`ipc.ts` + 错误码）
**Files:** Modify `app/frontend/src/ipc.ts`（`ParserColumn/ParserPluginInfo/ParserPluginsReply/ParserRow/ParserStats` + `parserPluginsList/parserRun/parserCancel`（双通道 + `onConsole` 照 canParse）；错误码 `plugin_not_found`/`plugin_bad_manifest` 进 `IpcErrorCode` 与中文表）；Modify `app/frontend/src/ipc.test.ts`（+2）。
**验证:** `npx tsc --noEmit` 净；`npx vitest run src/ipc.test.ts` 21 pass。

### Task 4. `ParserPanel`（新 `parserpanel.ts`）
**Files:** Create `app/frontend/src/parserpanel.ts`、`parserpanel.test.ts`（5）；Modify `app/frontend/src/styles.css`（`#parser-dock` 并入 `#can-dock` 的高度契约选择器）。
`RowStore`（纯数据累加器，同 FrameStore）+ `ParserPanel`：列模型由插件 `columns` 动态生成（首列固定「行号」）、自写虚拟滚动镜像 canpanel（`ROW_HEIGHT` + spacer + 可视窗 + rAF 合并重绘）、行点击回传 1-based 源行、`setStatus` 状态行；无过滤/导出/列显隐（spec §4）。CSS 复用 `can-*` 类，两表观感一致。
**Why:** canpanel 的列定义/过滤是 CAN 专用的，mf4-table 无虚拟化，均不可直接复用；取 canpanel 骨架裁剪是最小改动路径。
**验证:** `npx vitest run src/parserpanel.test.ts` 5 pass（列头动态渲染、批量 push 累积、行点击回传、状态行、destroy 后不再响应；rAF 合并用「等一个完整 rAF tick」的 flush 消除 jsdom 16ms 定时器抖动）+ `npx tsc --noEmit` 净。

### Task 5. dock 接线（`main.ts`）
**Files:** Modify `app/frontend/src/main.ts`（工具栏「解析」按钮；`buildParserDock()`（插件下拉 + 解析/停止 + 重载 + 清空 + 控制台区）；`toggleParserDock()`（与报文/测量 dock 三向互斥，收起即停跑）；`loadParserPlugins()`（按活动文档扩展名预选、错误汇总 toast + console.warn）；`applyParserColumns()` 切插件即重建 store/列；`onParserRunClick()`：parserToken 丢弃旧响应、onDone 出汇总文案；`appendParserConsole()` 200 行上限；handleClose 停跑）。
**Why:** 解析结果是一次性内存态，dock 隐藏/文档关闭都直接停跑，不做按文档缓存。
**验证:** `npx tsc --noEmit` 净；`npm test` 139 pass（21 文件）+ `npm run build` 通过。
**未验:** dock 实际交互（放示例插件解析、点行跳转）需 `npm run tauri dev` 人工过。

### Task 6. 文档收尾
**Files:** Modify `docs/features.md`（待做第 1 条注记更新：增量2 已交付、剩余 Wasm host 与表格过滤/导出增强）。
**验证:** 纯文档改动，未动代码，不涉及测试。

## 2026-10-30 — Plan 17 插件接口 增量1：声明式语法高亮插件（feat/plugin-syntax）

**依据：** spec `docs/specs/2026-10-30-plugin-syntax-design.md`；计划 `docs/plans/2026-10-30-plugin-syntax.md`。对应 `docs/features.md` 待做第 1 条（插件接口分三增量：声明式高亮 → QuickJS 数据解析 → Wasm host，本 plan 只做增量1）。

### Task 1. Rust 插件仓库（`plugins.rs` + `syntax_plugins_list`）
**Files:** Create `app/src-tauri/src/plugins.rs`（manifest 结构 `SyntaxPluginJson/SyntaxRuleJson`（serde camelCase + `deny_unknown_fields` 防字段拼错静默失效）、`validate()`（id/name/扩展名/关键词/lineComment 字符集与上限、tag 限 7 个枚举、规则 ≤32、文件 ≤256KB、目录 ≤64 个）、`load_syntax_plugins()`（文件名序、坏 JSON 单文件报错不阻塞、id 重复后到者跳过）、`plugins_dir()`、命令 `syntax_plugins_list`）；Modify `app/src-tauri/src/lib.rs`（mod + 注册）。
**Why:** 插件不执行任何代码，校验收在 Rust 侧（重逻辑原则）；正则编译校验放前端（JS `RegExp` 与 Rust `regex` 方言不同，见 Task 2）。
**验证:** `cargo test -p lumelog-app plugins::` 8 pass（合法 manifest、非法 tag、未知字段、坏 JSON 不阻塞其它、id 重复后文件名排序后到者跳过、extensions/keywords 上限、非 .json 忽略、目录缺失返回空）；`cargo test --workspace` 全绿 + `clippy --workspace --all-targets -- -D warnings` 净。

### Task 2. 前端插件注册表（`language.ts` + `ipc.ts`）
**Files:** Modify `app/frontend/src/language.ts`（`type ModeId = ModeChoice | \`plugin:${string}\``、`pluginModeId()`、`registerPluginLanguages()`（整表替换，坏正则单条跳过并回报 warning）、`autoModeId()`（内置扩展名优先、插件兜底）、`syntaxModes()`（内置 21 项 + 插件）、`resolveLanguage` 收 `ModeId` 并先查插件表）、`app/frontend/src/ipc.ts`（`SyntaxPlugin/SyntaxPluginsReply` + `syntaxPluginsList()`）；Modify `app/frontend/src/language.test.ts`（+3）、`app/frontend/src/ipc.test.ts`（+1 typeof）。
**Why:** Rust 只验 shape，JS 正则方言（如 lookahead）由前端 `new RegExp(src, "y")` 编译把关；插件模式 id 带 `plugin:` 前缀，与内置 id 空间不相交，状态栏与 `applyMode` 无需特判。
**验证:** `npx vitest run src/language.test.ts` 10 pass（插件解析出 `LanguageSupport` 且可 parse、AUTO 内置优先不被劫持、坏正则跳过且 warning 含插件名、`syntaxModes` 内置在前插件在后且空重载后消失、未知 `plugin:x` → null）+ `tsc --noEmit` 净。

### Task 3. 接线（`statusbar.ts` + `editor.ts` + `main.ts`）
**Files:** Modify `app/frontend/src/statusbar.ts`（`SyntaxModeOption`、`setSyntaxModes()` 重建下拉且保住仍在列表中的当前选中、`onMode` 回调签名放宽为 `(mode: string) => void`）、`app/frontend/src/editor.ts`（`setMode(mode: ModeId | null)`、首语言走 `autoModeId(opts.ext)`）、`app/frontend/src/main.ts`（`initSyntaxPlugins()`：启动加载 + `pluginsInFlight` 合并连点 + 汇总 toast/console.warn，末尾 bootstrap 处 `void initSyntaxPlugins()`；`applyMode` 收 string 并在 AUTO 时走 `autoModeId`；命令面板新增「重载语法插件」palette-only 条目，不占 keymap 动作位）；Modify `app/frontend/src/statusbar.test.ts`（+1）。
**Why:** 插件加载晚于 `Statusbar` 构造，下拉必须可重建；重载只影响新开文档与手动切换，不重建已打开编辑器。
**验证:** `npx tsc --noEmit` 净；`npm test` 132 pass（20 文件；新增 statusbar 重建下拉/保选中/回传 `plugin:<id>`）+ `npm run build` 通过。

### Task 4. 文档收尾
**Files:** Modify `docs/features.md`（待做第 1 条加增量1 交付注记：manifest 路径、字段摘要、增量2/3 边界）。
**验证:** 纯文档改动，未动代码，不涉及测试。
**未验:** 插件实际高亮观感需 `npm run tauri dev` 放一个示例 JSON 人工过（自动化只覆盖到解析器挂载与下拉）。

## 2026-10-30 — Plan 16 编码扩展：EUC-JP / EUC-KR / KOI8-R / KOI8-U（feat/encoding-extension）

**依据：** spec `docs/specs/2026-10-30-encoding-extension-design.md`；计划 `docs/plans/2026-10-30-encoding-extension.md`。对应 `docs/features.md` 待做第 3 条（原文把 Big5、Shift-JIS 列为缺口是笔误，两者 v1 已交付）。

### Task 1. `EncodingLabel` 四标签 + 检测双 pass
**Files:** Modify `crates/lumelog-core/src/encoding/mod.rs`、`crates/lumelog-core/tests/encoding_proptest.rs`。
`EncodingLabel` 加 `EucJp/EucKr/Koi8R/Koi8U`（`as_str`/`from_label` 含 `EUCJP` 等无连字符别名/`encoding()` 映射到 `encoding_rs::EUC_JP|EUC_KR|KOI8_R|KOI8_U`）；`label_from_encoding` 加四个映射（含 `KOI8_U→Koi8U`——chardetng 对俄文一律报 KOI8-U，俄文字母在两变体码位相同，解码正确）；`detect()` 在带 `cn` hint 判定为 `Gbk|Gb18030` 时用同一 head 无 hint 复核，且只接受 `EucJp|EucKr`。proptest `label_pool` 补 4 项。
**Why:** 探针实测（`guess_assess`，1KB 样本）：EUC-KR/EUC-JP 带 `cn` hint 判成 GBK、无 hint 判对且 plausible；GBK/GB18030 中文样本无 hint 仍是 GBK——双 pass 不会翻转中文判定。KOI8 系 hint 判定即 KOI8-U，直接映射即可，不加则俄文永远兜底 GB18030。`bom_len`/`has_bom`/`IncrementalDecoder`/`encode_all` 零改动：四编码均无 BOM、无 U+FFFD 码点，走既有 legacy `?` 分支。
**验证:** `cargo test -p lumelog-core` 95 pass（新增 roundtrip×4、emoji→单字节 `?` 无 `&#` 引用、别名 6 组、EUC-KR/EUC-JP 检出、俄文检出 KOI8-U；既有 `detect_gbk_sample_is_chinese`/`detect_falls_back_when_result_not_in_label_set` 不回归）；`cargo test --workspace` 全绿（app 117 / can 23 / core lib 95 / proptest 8）；`cargo clippy --workspace --all-targets -- -D warnings` 净。检测判定过程曾以临时探针 `tests/encoding_detect_probe.rs` 验证，结论记录于 spec §1.1 后删除，不入仓。

### Task 2. 前端编码菜单 13 项
**Files:** Modify `app/frontend/src/encoding-menu.ts`（`ENCODINGS` 末尾追加 EUC-JP/EUC-KR/KOI8-R/KOI8-U，display = label）、`app/frontend/src/ipc.test.ts`（`encoding_menu_labels_match_spec` 断言 9 → 13 项）。
**Why:** 状态栏下拉、`changeEncoding` 链路都从 `ENCODINGS` 派生，加表即可全链路生效；label 直传 `EncodingLabel::from_label`（Rust），三个无连字符别名只做容错不进菜单。
**验证:** `npm test` 127 pass（20 文件，断言更新无新增用例）+ `npm run build` 通过；Rust 侧本轮无改动。

### Task 3. 文档收尾
**Files:** Modify `docs/specs/2026-10-24-lumelog-v1-design.md`（§5.1 支持集合补 4 项 + 检测补充说明）、`docs/features.md`（条目移入已交付，注明 Big5/Shift-JIS 原为笔误、UTF-32 不做）、`docs/test-checklist.md`（§2 冒烟补 EUC-KR/EUC-JP/KOI8 样例行）。
**验证:** 纯文档改动，未动代码，不涉及测试。

## 2026-10-30 — Plan 15 语法高亮语言扩展（feat/syntax-languages）

**依据：** spec `docs/specs/2026-10-30-syntax-languages-design.md`；计划 `docs/plans/2026-10-30-syntax-languages.md`。对应 `docs/features.md` 待做第 2 条。

### Task 1+2. 注册表 + 语言包（`language.ts`）
**Files:** Modify `app/frontend/package.json`/`package-lock.json`（新增 `@codemirror/lang-javascript|python|html|css|xml|markdown|sql|rust|java` 与 `@codemirror/legacy-modes`）、`app/frontend/src/language.ts`；Create `app/frontend/src/language.test.ts`（7）。
新增 `AVAILABLE_MODES`（21 项，id 沿用 auto/plain/log/ini/json/c）、`MODE_LABELS`、`EXT_MODES`（扩展名→id）、`modeIdForExt(ext)`（同步纯函数，未知 → null）、`resolveLanguage(mode)`（`import()` + `Map` 缓存 + `inFlight` 合并并发）；`log`/`ini` 仍走本地 `StreamLanguage` + `streamHighlight`，lezer 语言与 5 个 legacy 流模式都靠 `editor.ts` 已挂载的 `defaultHighlightStyle`（fallback）着色，故未新增 `HighlightStyle`。
**Why:** 原实现把模式 id 当扩展名用（`applyMode` 传 `mode` 给 `modeForExtension`），加语言就得让两个字符串集合保持一致；静态引入 9 个语法包会把首屏主 chunk 从 623KB 推到 1MB 量级，与轻量定位冲突。
**验证:** `npx vitest run src/language.test.ts` 7 pass（扩展名映射 39 组、`AVAILABLE_MODES` 除 auto/plain 外全部解析出 `LanguageSupport` 防注册表漂移、`plain/auto/null` → null、二次调用同一实例、`javascript` 与 `typescript` 用 `interface Foo { a: number }` 的 error 节点区分方言）。

### Task 3. `editor.ts` + `main.ts` 异步接线
**Files:** Modify `app/frontend/src/editor.ts`（`buildState` 改收已解析的 `LanguageSupport | null`；`openEditable` 先 `await resolveLanguage(modeIdForExt(opts.ext))`；`setMode(mode: ModeChoice | null)` 内部异步，`++modeToken` 守卫旧回包，`destroy()` 置 `destroyed` 后不再 dispatch）、`app/frontend/src/main.ts`（`applyMode`：`auto` → `modeIdForExt(extOf(path))`，`plain` → null，其余传 id）；Create `app/frontend/src/editor.test.ts`（3，`vi.mock("./ipc")`）。
**Why:** 动态加载让模式切换变异步，连切语法时先发的慢包必须输给后发的包；view 已销毁后 dispatch 会抛错。
**验证:** `editor.test.ts` 3 pass（按扩展名挂载 python；python→json 连切最终为 json、再切回 python；destroy 后回包的语言不生效）。

### Task 4. 状态栏显示名 + 收尾
**Files:** Modify `app/frontend/src/statusbar.ts`（`mkSelect(cls, labels, values?)` 支持显示名与取值分离，下拉用 `MODE_LABELS`，`onMode` 回传 id）、`app/frontend/src/statusbar.test.ts`（+1）、`docs/features.md`（语法条目移入已交付）。
**Why:** 20 种语言再靠 `toUpperCase()` 生成显示名会出现 `JAVASCRIPT` 这类文本；取值必须仍是模式 id。
**验证:** `tsc --noEmit` 净；`npm test` 127 pass（20 文件，基线 116/18）；`npm run build` 通过，主 chunk `dist/assets/index-D8No6Jyb.js` 506.9KB（基线 623.3KB，json/cpp 一并拆出），语言落在独立 chunk（legacy 模式为具名 `yaml/toml/shell/diff/powershell-*.js`）。Rust 侧本轮零改动（`git status` 只有 `app/frontend` 与 `docs`），`cargo test --workspace` / `clippy` 无回归面。
**未验:** 各语言实际着色效果需 `npm run tauri dev` 人工过一遍（jsdom 无排版，只验了解析器与挂载）。

## 2026-10-26 — docs：定位改为 Notepad++ 替代编辑器

**Files:** Modify `docs/claude.md`。
**Why:** 定位表述更新：由“下一代日志与 CAN 报文查看器”改为“跨平台开源 Notepad++ 替代编辑器”——保留 Notepad++ 的操作习惯、宏能力与文本体验，原生内置工控大日志、多编码、DBC/CAN 报文解析模块，面向嵌入式、HIL 测试工程师及普通开发者；删除简介引言行，定位只在「项目定位」一节保留一处。
**验证:** 纯文档措辞修改，未动代码，不涉及测试。

## 2026-10-26 — docs：立项表述改为对标超越 Notepad++

**Files:** Modify `docs/claude.md`。
**Why:** 定位表述按项目方口径替换：不是“另一个 Notepad++”，而是面向工控和嵌入式测试的下一代日志与 CAN 报文查看器，通用编辑能力对标 Notepad++，大日志、工控编码、DBC 解析、跨平台超越之；简介行同步。原“通用文件编辑查看器”句式废止，功能清单指针不变（features.md 已单列）。
**验证:** 纯文档措辞修改，未动代码，不涉及测试。

## 2026-10-26 — docs：插件接口条目补充冷门语言解析要求

**Files:** Modify `docs/features.md`。
**Why:** 待做的插件接口条目原句只写“支持用户扩展解析器”，未覆盖冷门语言场景；补明冷门编程语言的高亮/解析与冷门日志、报文格式都应能经插件补入，不改主程序。
**验证:** 纯文档措辞修改，未动代码，不涉及测试。

## 2026-10-26 — docs：定位改通用文件编辑查看器，功能清单拆到 features.md

**Files:** Modify `docs/claude.md`；Create `docs/features.md`。
**Why:** 产品定位从“工业调试专用轻量编辑器”改为“通用文件编辑查看器”，新增 PDF/PPT/docx/xlsx 等常见文档格式查看与导出的规划条目；`claude.md` 原规划功能清单（30 余行）整体迁到 `docs/features.md`（按已交付/待做分节，`↳` 交付细节原样保留），`claude.md` 只留指针，正文收敛为定位/痛点/技术栈/同类差异/Git 约定。
**验证:** 通读两文件确认清单内容无删改、`[features.md](features.md)` 相对链接指向存在文件；纯文档改动，未动代码，不涉及测试。

## 2026-10-26 — Plan 14 Roadmap MF4 波形（增量2，feat/roadmap-mf4-chart）

**依据：** spec `docs/specs/2026-10-26-roadmap-mf4-chart-design.md`；计划 `docs/plans/2026-10-26-roadmap-mf4-chart.md`。

### Task 1+2. `mf4_series` 桶化 + 命令 + IPC
**Files:** Modify `app/src-tauri/src/mf4.rs`（`MAX_SERIES=8`/`MAX_BUCKETS=4000`；`Mf4SeriesJson{has_time_axis,from_sec,to_sec,bucket_sec,times,series}`、`Mf4SeriesEntryJson{index,name,unit,degraded,min,max,first,last}`（均 `Vec<Option<f64>>`）；`series()`：同 `records` 规则（同数据组 + `is_time_channel()`），文本/未知类型→`invalid_argument`；窗口未给时用主通道首末样本解析；`aligned_signal_chunks(&[master]+channels, 65536)` 逐块取同下标值，桶号 `((t-from)/bucket_sec).floor().clamp(0,n-1)`，逐桶累积 min/max/first/last，空桶 `None`；全 `null` + `has_time_axis=false` 的无时间轴路径）；`commands.rs`/`lib.rs`（`mf4_series` 命令，`buckets` 缺省 2000）；`ipc.ts`/`ipc.test.ts`（`Mf4Series/Mf4SeriesEntry` + `mf4Series`，typeof 8）。
**Why:** 波形必须保尖峰且不让整信号进前端；`aligned_signal_chunks` 提供同下标对齐，比逐通道重扫稳定；假定“同组等采样”已在注释写入（混采样留后续 `Raster`）。
**验证:** `cargo test -p lumelog-app mf4::` 12 pass（新增桶数/包络/first-last/空桶/尖峰 999 保留/跨组与文本拒绝）；`cargo test --workspace` app 117 / core 89 / can 23；`clippy --workspace --all-targets -- -D warnings` 净；`npm test` 103 + `run build` 通过。

### Task 3. 波形数学（`mf4-chart.ts` 纯函数）
**Files:** Create `app/frontend/src/mf4-chart.ts`（`clampWindow`（全范围/最小窗宽 `fullSpan/1e6` 或 `1e-6`）、`zoomAt`（锚点时间不变）、`panBy`（两端夹紧）、`bucketIndex`（`t==end` 归末桶、零宽窗口归 0）、`nearestBucket`（跳空桶，全空返回 -1）、`xOf`、`yOf`（归一化；min==max 落中线、NaN→NaN））、`mf4-chart.test.ts`（6）。
**Why:** 交互手感靠窗口数学，先以纯函数钉住（jsdom 无真像素，像素级留给手工验）。
**验证:** `npm test` 109 pass（17 文件）+ `run build` 通过。

### Task 4. `Mf4Chart`（双层 canvas + 交互）
**Files:** Modify `app/frontend/src/mf4-chart.ts`（`Mf4Chart`：base + overlay 两张 canvas、tooltip `div`；`ResizeObserver` + DPR（`setTransform` 只设一次）、`requestAnimationFrame` 合并重绘；底图网格/时间刻度/每序列 min–max 竖线 + 桶间细连线（10 色、归一化）；工具条层级：`setData/setWindow/setBusy/onWindowChange/destroy`；wheel（`passive:false`、1.25^…、光标为锚点）、pointer 拖拽（`setPointerCapture`）、dblclick 复位、hover 十字线 + tooltip（`nearestBucket` 的首值）；`getContext` 为空时安全降级（不抛错、只回调））、`styles.css`（`.mf4-chart*`：相对布局、叠层 canvas、tooltip；`mf4-chart.dom.test.ts`（3：双层与尺寸、60ms 节流只触发一次 + dblclick 复位、destroy 后不再响应）。
**Why:** hover 高频重绘只碰 overlay；DPR 只 `setTransform` 一次避免累积缩放；jsdom 无 canvas 实现时保证交互测试仍可跑。
**验证:** `npm test` 112 pass（18 文件）+ `run build` 通过。

### Task 5. dock tab + `main.ts` 注入
**Files:** Modify `mf4-ui.ts`（「表格 / 波形」tab；`hasTimeAxis===false` 时波形 disabled；波形模式只允许数值通道且上限 8，切回表格恢复 32；`setSeries()` 记住全范围并重置窗口；图表窗口文案 + 「应用到表格」（写入起/止并重查）+ 「复位」；`emitSeries()` 在无窗口时发 `null` 窗口让后端自行解析全范围；`destroy()` 释放图表监听）、`mf4-ui.test.ts`（+2：切 tab 发 `onSeriesQuery`/文本通道禁用/应用到表格后 `onQuery` 窗口一致；无时间轴 tab disabled）、`main.ts`（`mf4Series` 注入 + `mf4SeriesToken` 丢弃旧响应 + `closeMf4` 递增 token 并 `destroy()`）、`ipc.ts`（`onSeriesQuery` 允许 `null` 窗口）。
**Why:** 波形窗口独立（缩放不惊动分页），仅在「应用到表格」时同步；首帧无窗口时由后端解析全范围，避免前端猜 0/0。
**验证:** `npm test` 114 pass（18 文件）+ `run build` 通过（49 modules）。

### 终审修复波（0 Critical / 5 Important / 5 Minor）
**Files:** `mf4-chart.ts`（**I1** `setData` 不再重定义 `full`/重置窗口：首帧才定全范围，「响应只换数据」——修正首次缩放后无法再缩小/平移/双击复位；`setFullRange` 独立；底图按 `data.from/from.to` 裁剪并把 `min/max` 均缺的桶断线（不再画成`lo/hi` 满幅假尖峰）；**I3** 滚轮方向改为“向上=放大”（`1.25^(deltaY/100)`）；**I2 配套** 新增 `pointercancel`/`lostpointercapture` 复位拖拽）；`mf4-ui.ts`（**I2** 切波形时把文本/未知/时间主通道从选择中剔掉并重新渲染，`emitSeries` 再次过滤防混选；状态行在波形模式显示 `已选 N/8`；tab 移到工具栏上方，两种模式不跳动）；`mf4.rs`（**I4** 仅在 `from/to` 缺一才扫主通道首末；**I5** 主通道与选择锁定到同一 `channel_group_index`（对齐假定与注释一致）；**I6** 窗口倒置/空 → 返回空桶而非虚构 span）；`styles.css`（`#mf4-dock{overflow:hidden}`、`.mf4-chart(-host){min-height:0}`）。新增前端测 2（缩放响应不改变全范围 + dblclick 仍回全范围；表格选文本通道→切波形自动剔除），Rust 12 测不回归。
**Why:** I1 是导航性失效（只能单向放大）；缺值桶会被画成满幅假噪声；混选文本会令整次波形请求 `invalid_argument`。
**Parked:** 图例/点击隐藏序列、波形页的 `⚠` 与“无样本”文案、tooltip 用桶首值而标题是光标时刻（最多差一桶）、`buckets` 固定 2000 而非按像素宽、resize 后不重采样。
**验证:** `cargo test -p lumelog-app mf4::` 12 pass + `clippy --workspace --all-targets -- -D warnings` 净；`npm test` 115 pass（18 文件）+ `run build` 通过。

### Parked 清理（波形 5 项）
**Files:** `app/frontend/src/mf4-chart.ts`（**M1** 图例：每序列颜色点 + 名称，点击显隐（`hidden` 集合、断线重绘）；`degraded` 序列前缀 `⚠` 与 tooltip 说明；**M2** 底图在“无可见有限值”时居中显示「无可绘制数据」（全 `degraded`）或「该区间无样本」；**M3** tooltip 标题改为**桶时间**（与实际取值 `first[b]` 同源，不再“光标时刻 vs 桶首值”错位）；隐藏序列不进 tooltip；**M4/M5** 新增 `onResize`（宽度变化 >10% 才触发）、`destroy` 清防抖定时器）；`mf4-ui.ts`（`seriesBuckets()` = `clamp(宽 × dpr × 2, 200, 4000)`；`queueSeries()` 150ms 防抖用于 resize/tab 切换；`destroy` 清定时器）；`styles.css`（`.mf4-chart-legend*`、色点、隐藏态划线）。
**Why:** 图例是 spec §4 明确项；`⚠`/「无样本」是 spec §5 要求的状态面；tooltip 时间必须与取值同源；桶数按像素宽才能兼顾清晰度与体积；窗口缩放后旧桶宽已失真。
**验证:** `npm test` 116 pass（18 文件，`mf4-chart.dom` +1 图例显隐、`mf4-ui` 增加 buckets 区间断言）+ `run build` 通过；Rust 侧本轮无改动（mf4 12 测 + clippy 上轮已验证）。

## 2026-10-26 — Plan 13 Roadmap MF4 测量数据查看器（feat/roadmap-mf4-viewer）

**依据：** spec `docs/specs/2026-10-26-roadmap-mf4-viewer-design.md`；计划 `docs/plans/2026-10-26-roadmap-mf4-viewer.md`（增量1：元数据 + 通道 + 分页记录表 + CSV 导出；波形为增量2）。

### Task 1. MSRV 1.89 + `falcon_mdf` + MF4 会话/通道
**Files:** Modify `Cargo.toml`/`app/src-tauri/Cargo.toml`（`rust-version 1.87→1.89`；`falcon_mdf = { version = "0.7", default-features = false, features = ["mmap"] }`）、`Cargo.lock`、`.github/workflows/ci.yml`（工具链注释）；Create `app/src-tauri/src/mf4.rs`（`Mf4Sessions`（id→会话，上限 4）、`open_session/close_session/list_channels`、`Mf4InfoJson/Mf4ChannelJson/Mf4ChannelsPage/Mf4Kind`、错误分类 `mf4_not_mf4/mf4_encrypted/mf4_unsupported/io`、`kind_of` 按 `data_type` Debug 字串判定）、`commands.rs`（3 个 `#[tauri::command]`）、`lib.rs`（`mod mf4` + `manage` + 注册）；`MAX_COLUMNS/MAX_LIMIT/session.file/path` 临时 `#[allow(dead_code)]`（T2/T3 使用后剥）。
**Why:** MF4 不能走文本/行索引链路 → 独立会话 + dock；`falcon_mdf` 类型只出现在 `mf4.rs`（可换库）；只开 `mmap` 避免 rayon/arrow 依赖面；打开失败一律归为结构错误（非真 I/O），避免把“不是 MF4”误报为 io。
**验证:** `cargo test -p lumelog-app mf4::` 3 pass（夹具用 `Mf4Writer` 现写 MF4：计数/主通道/名称、大小写不敏感搜索+分页+越界空页、非 MF4 → `mf4_not_mf4`、会话上限 4 → `invalid_argument`、close 幂等、未知会话 `unknown_mf4`）；`cargo test --workspace`（app 108 / core 89 / can 23）+ `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 2. `mf4_records`（时间窗 + 分页 + 单元格映射）
**Files:** Modify `app/src-tauri/src/mf4.rs`（`select_rows` 流式主通道得 `(样行, 毫秒)` 对并计数 total（降序就地反转，再 skip/take）；`cells_for_rows` 逐块只取本页行下标（内存与页大小成正比）；`cell_of` 映射数值/文本变体，失效样本与读失败→`Null`，整列不可读→`degraded`；MDF 把 bool 存成整数，按 `kind_of` 回报 `Bool`；`records` 校验选择非空/≤32 列/limit≤1000/from≤to/排序仅 timeAsc|timeDesc）；`commands.rs` + `lib.rs`（`mf4_records` 命令与注册）；`path` 字段保留 `#[allow(dead_code)]`（T3 导出使用）。
**Why:** spec §3/§5：不能把整信号物化到内存才能撑住大文件；`total_rows` 需要全窗口计数，因此只对时间轴做一遗扫描（页数据按块取）；排序语义固定为时间跳页、列仅本页（列排序需物化，本增量不做）。
**验证:** `cargo test -p lumelog-app mf4::` 5 pass（新增窗口裁剪/分页与 total/timeDesc 反向/列顺序按选择/失效样本 Null/六类参数拒绝）；`cargo test --workspace` app 110 全绿；`cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 3. `mf4_summary` + CSV 导出 + 命令注册
**Files:** Modify `app/src-tauri/src/mf4.rs`（`Mf4SummaryJson`/`Mf4GroupJson`（按 `channel_group_index` 聚合）/`Mf4ExportProgress`/`Mf4ExportStats`；`summary()`（version/path/comment/计数/组聚合/events+attachments 数量）；`export_csv()`（500 行/批，复用 `cells_for_rows` 与表格同一映射；`timeMs` 头 + `名 (单位)` 头；文本引号加倍；取消保前缀；tmp→rename；进度每 5 万行）；`csv_cell/csv_header`）；`commands.rs`（`mf4_summary`、`mf4_export_csv`（跨 `Mf4Sessions`+`AppDocs` 两托管态，worker 内取会话、`begin_search`/`end_search`、`catch_unwind` 仍需 `on_done`）、`mf4_export_cancel`）；`lib.rs` 注册；新增 3 测（summary 计数/组、导出与表格一致+引号转义、空选择与取消前缀）。
**Why:** 导出与表格必须同源映射（都走 `cells_for_rows`），否则两边会漂移；取消仍转成可读前缀（tmp→rename）。
**偏差（记录）:** `falcon_mdf` 的 `Event`/`Attachment` 字段未深入封装，summary 先只给 events/attachments 的数量，事件明细留后续；列排序仅 timeAsc/timeDesc（列序在 UI 本页内做）。
**验证:** `cargo test -p lumelog-app mf4::` 8 pass（+3）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 4. 前端 IPC（MF4）
**Files:** Modify `app/frontend/src/ipc.ts`（`Mf4Kind/Mf4Info/Mf4Channel/Mf4ChannelsPage/Mf4Cell/Mf4Column/Mf4Rows/Mf4Summary/Mf4ExportStats` + `mf4Open/Close/Channels/Summary/Records/ExportCsv/ExportCancel`）；`ipc.test.ts`（+1 typeof 7）。
**Why:** 单元格改为 `#[serde(untagged)]`（`number|string|boolean|null`），前端零解包；Rust `Mf4CellJson` 同步改为 untagged（原 camelCase 会变 `{number:10}`，已在 T4 一并修正）。
**验证:** `npm test` 95 pass（ipc 17→18）+ `run build` tsc 通过。

### Task 5+6. MF4 dock（表格 + 通道面板 + 分页/时间窗/导出）+ main 接线
**Files:** Create `app/frontend/src/mf4-table.ts`（`cellText`：null→`—`/bool→`✓`/浮点修剪；表头带单位，`degraded` 加 `⚠`+tooltip；**列排序仅本页**、点击切换升降；页内直接渲染，不分页内部虚拟化）、`mf4-ui.ts`（`Mf4View`：通道搜索 150ms 防抖、复选上限 32、时间窗校验（起>止拒）、排序/每页 100/500/1000、上一页/下一页按 `totalRows` 禁启、导出入参仅选列+窗口、回调注入）、`mf4-table.test.ts`（3）、`mf4-ui.test.ts`（4）；Modify `styles.css`（`.mf4-*`）、`main.ts`（工具栏“测量”、面板“测量数据…”、`#mf4-dock`、`mf4Open/mf4Channels/mf4Summary` 并行、`runMf4Query` 带 token 丢弃旧响应、导出走 `save()`+`mf4ExportCsv` 进度/完成、关闭释放会话、与 CAN dock 双向互斥）。
**Why:** 一页 ≤1000 行，直接渲染比自写虚拟化更可验证（jsdom 与真实行为一致）；列排序只在本页内（后端只做时间序跳页）避免物化整窗。
**偏差/Parked:** dock 未内置“取消导出”按钮（`mf4_export_cancel` 已有，UI 仅可在完成后解除）；列选择上限/分页默认值与 spec 一致；会话多开（>1）UI 仅保留最后一个（后端支持 ≤4，UI 先简化为单会话）。
**验证:** `npm test` 102 pass（16 文件）+ `run build` tsc 通过（48 modules）。

### 终审修复波（全分支审查：1 Critical / 6 Important / 6 Minor）
**Files:** `app/src-tauri/src/mf4.rs`（**C1** `Mf4ChannelJson` 增 `dataGroup`；`records`/`export_csv` 改为“同一 `data_group_index` + `is_time_channel()`”取时间主通道，跨数据组选择直接 `invalid_argument`；**I2** 降序页改为先升序取单元格再按 `slot_of` 回排，并在 `select_rows` 用 `last_needed = total - start` 作为扫描终点（避免跨分块下溢/尾部空值）；**I3** `select_rows` 改两遍流式（第一遍只计数、第二遍用 `HashMap<position, slot>` 只收本页），无时间轴用 `signal_chunks` 累加长度，不再整信号入内存；**I4** 导出先快照 `Arc<Mutex<Mf4File>>` 并 `drop(session 锁)`，长写入不再阻塞表格/概览）；`app/frontend/src/mf4-ui.ts`（`query()` 增加 `busy` 短路；“已选 N/32”只在数量变化时覆盖状态区，不再掩盖“读取中/导出 N 行”）；`main.ts`（`mf4Opening` 防重入；打开后 `channels/summary` 失败时 `mf4Close(opened)` 回收会话槽；超 2000 通道时状态区提示“仅加载前 N，请用搜索缩小范围”）；`styles.css`（**I5** `#mf4-dock` 补 `flex:0 0 auto;max-height:55%;min-height:0;display:flex;flex-direction:column`，`.mf4-view` 改 `flex:1 1 auto` 去掉 `height:100%`，表格/通道列重新可滚动）；新增回归测（两个数据组 → 跨组选择 `invalid_argument`、单组仍播 3 行）。
**Why:** C1/I2 会静默给出错误数据；I3 在百万样本级文件上每翻页重整时间窗；I5 导致页只能看到视口内几十行。
**Parked:** Minor 的 `Bytes/Complex → degraded`、导出 UI 取消按钮、`summary` 组合并同号组、无时间轴表头/行数一致性、跨分块降序/无时间轴/`busy` 点复选框等测试（详见 `docs/reviews/2026-10-26-mf4-review-findings.md`）。
**验证:** `cargo test -p lumelog-app mf4::` 9 pass；`cargo test --workspace`（app 114 / core 89 / can 23）、`cargo clippy --workspace --all-targets -- -D warnings` 净；`npm test` 102 pass；`run build` 通过。

### Parked 清理（终审 Minor 全部消化）
**Files:** `app/src-tauri/src/mf4.rs`（**M1** `cell_of` 返回 `Option`，`Bytes/VarBytes/Complex/Array` 等无单元格类型时 `cells_for_rows` 置 `degraded=true`（整列 `—` + `⚠`）；**M3** `Mf4GroupJson` 增 `dataGroup`，`summary` 以 `(data_group, group)` 为键并按序排序；**M4** 无时间轴导出改以“文件第一个通道”为行数基准（与表格一致），CSV 表头切为 `index`；新增 1 测（VLSD bytes → `degraded` + 全 `null`））；`ipc.ts`（`Mf4Summary.groups` 增 `dataGroup`）；`mf4-ui.ts`（工具栏新增「取消导出」按钮，`setExporting` 才武装；`summary` 类型同步 `dataGroup`）；`main.ts`（记录 `mf4ExportReq`，`onDone`/错误/`closeMf4` 都会清并/或发 `mf4ExportCancel`；新增 1 jsdom 测：`busy` 时不发查询 + 取消按钮只在导出时可用）。
**Why:** 无单元格类型的列原先整列空且无提示；导出与表格在无时间轴文件上可能行数不一致；取消导出是 spec §4 明确项。
**剩余（需真实样本/大夹具，单独立项）:** 跨 4 MiB 分块降序回归、无时间轴夹具、>`2000` 通道搜索走后端分页。
**验证:** `cargo test -p lumelog-app mf4::` 10 pass，`cargo clippy --workspace --all-targets -- -D warnings` 净；`npm test` 103 pass（16 文件）+ `run build` 通过。

## 2026-10-25 — Plan 12 Roadmap QuickJS 脚本引擎（feat/roadmap-script）

**依据：** spec `docs/specs/2026-10-25-roadmap-script-design.md`；计划 `docs/plans/2026-10-25-roadmap-script.md`（Roadmap：脚本管理器 + 逐行变换 + 控制台）。

### Task 1. MSRV 1.87 + `rquickjs` + 脚本存储/工具
**Files:** Modify `Cargo.toml`/`app/src-tauri/Cargo.toml`（`rust-version 1.79→1.87`、`rquickjs = "0.14"`；QuickJS C 经 cc 编译成功）、`Cargo.lock`；Create `app/src-tauri/src/script.rs`（`ScriptEntry/Console/Progress/Stats`、名字校验 `^[A-Za-z0-9_-]{1,64}\.js$`、`scripts/` list/read/write/delete（tmp→rename）、有界控制台 200×4096）；Modify `lib.rs`（`mod script`）、`commands.rs`（`config_path` 提升 `pub(crate)`）、`crates/lumelog-can/src/source/mod.rs`（MSRV 1.87 使 clippy 新增 `manual_is_multiple_of` → `is_multiple_of`）。
**Why:** 沙箱引擎与脚本存储都在 app 侧，不污染 core；脚本名只允许安全字符防目录穿越；保存原子化。MSRV 提升是 rquickjs 0.14 的硬要求（当前工具链 1.99）。
**验证:** `cargo test -p lumelog-app script::` 3 pass（名字 8 类非法/合法、存储 roundtrip+无 tmp 残留、控制台单条截断+200 条后仅一条“已截断”）+ `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 2. `run_script` QuickJS 运行器
**Files:** Modify `app/src-tauri/src/script.rs`（`Runtime` 64MiB/1MiB 栈 + interrupt（取消或 300s）；console shim 在 JS 侧 `JSON.stringify` 后回传；eval 一次→`onLine`；`DocLines` 逐行、三态返回、ctx `{lineNumber,index,path}`；输出 UTF-8/LF、4GiB 上限；tmp→rename（脚本错误保留前缀；I/O 错误删 tmp）；超时/取消/错误区分）；新增 8 测。
**Why:** spec §2/§3；只有 `DocLines` 分块读取才能保证 GB 级恒定内存；JS 侧格式化避免复杂值封送；`cancelled` 与“超时/脚本错误”分开上报。
**验证:** `cargo test -p lumelog-app script::` 11 pass（三态/行号/抛错前缀/缺 onLine/非法返回/UTF-8/console 三级别+对象 JSON/无 tmp 残留）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 3. 脚本命令 + 注册
**Files:** Modify `app/src-tauri/src/commands.rs`（`same_path`（canonicalize）；`script_list/read/write/delete`；`script_run`（doc 存在/目标≠源→`invalid_argument`；`begin_search`+`spawn_blocking`+`run_script`，console/progress/done 三 Channel）；`script_cancel`；测试 `same_path`）；`lib.rs`（注册 6 命令）；`script.rs`（除模块 allow；`RunStop::Io` 真正使用：写盘失败删 tmp 并返回错误）；新增 `stats_camel_case` 测试。
**Why:** 目标与源同路径会覆盖正在读的文档，prologue 拒绝；取消复用通用 registry；DTO camelCase 直接由 script 类型序列化。
**验证:** `cargo test -p lumelog-app script` 13 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 4. 前端 IPC（脚本）
**Files:** Modify `app/frontend/src/ipc.ts`（`ScriptEntry/Console/Progress/Stats` + `scriptList/Read/Write/Delete/Run/Cancel`，Run 三 Channel）；`ipc.test.ts`（+1 typeof 6 个）。
**Why:** `script-ui` 不直接依赖 ipc，由 main 注入回调；与现有双/三 Channel 包装形状一致。
**验证:** `npm test` 90 pass（ipc 16→17）+ `run build` tsc 通过。

### Task 5. `script-ui.ts` 脚本管理器模态
**Files:** Create `app/frontend/src/script-ui.ts`（列表/新建模板/删除/保存/textarea+Tab/控制台/运行+取消/状态行；未保存确认；运行中锁编辑；`inert` 背景）；`script-ui.test.ts`（4 个 jsdom：列表选中读取、非法名+新建模板、保存调用、无文档禁用+运行流控制台）；`styles.css`（`.script-*`）。
**Why:** 回调注入使模态不依赖 ipc、可 jsdom 测；名字规则与服务端一致；运行中禁止切脚本/编辑。
**验证:** `npm test` 94 pass（14 文件，script-ui 4）+ `run build` tsc 通过。

### Task 6. `main.ts` 脚本接线
**Files:** Modify `app/frontend/src/main.ts`（`scriptModalOpen`与快捷键短路；工具栏“脚本”+面板“脚本…”；`openScriptManager` 注入 list/read/write/remove/chooseTarget(save 默认 `<doc基名>.out.txt`)/run(解析当前 docId)/cancel/confirm(ask)；与 keymap 模态互斥）。
**Why:** spec §4；模态保持纯净（仅回调），文档不存在时运行 reject；两模态不能同时抢键盘/inert。
**验证:** `run build` tsc 通过（46 modules）+ `npm test` 94 pass。

### 终审修复波（CodeReview 全分支 0 Critical / 5 Important / 10 Minor）
**Files:** `script.rs`（**I1** 最终 `writer.flush()` 错误不再被吞 → 删 tmp 并报“写入输出文件失败”，避免静默丢尾；**I2** 新增 `RunStop::Cancelled`，`onLine` 调用失败时先判 `cancel()`/超时，避免“已取消/300s 超时”退化为 `interrupted` 错；**I3** `const/let onLine = () => …` 现代写法可用（global lexical binding 不在 globalThis，补一次求值）；**I5** `read_script_in` 1 MiB 上限 + 剥 UTF-8 BOM + `from_utf8_lossy`（外部编辑器存 ANSI/GBK 不再直接失败）；输出 tmp 名加纳秒防碰撞）；`commands.rs`（**I4** `run_script` 包 `catch_unwind(AssertUnwindSafe)`，panic 时仍发 `on_done` 并 `end_search`）；`script-ui.ts`（**M1** 删除重复 `box.append`；**M2** 列表补修改时间；**M3** Esc 关闭 + 初始聚焦关闭钮，监听器在 close 里摘除）；`main.ts`（**M10** `openScriptModal` 抛错时回滚 `scriptModalOpen`，否则全局快捷键永久失效）；新增 2 测（const 箭头函数、BOM/非 UTF-8 读取）。
**Why:** I1/I2/I4 均为低概率但后果不可恢复（静默丢数据 / 取消语义丢失 / 模态永久锁死）；I3/I5 直接影响可用性（现代写法与中文 Windows 记事本）。
**Parked（本次未修，已记录）：** M4 模态 `String(e)` 未全走 `.zh`；M5 测目录用 PID 命名（改用 tempfile）；M7 返回串内含 `\n` 不归一（文档语义：一行可拆多行）；M8 Windows 保留设备名黑名单；三平台 CI 矩阵需开 PR 验证 `rquickjs-sys` 在 macOS/Linux 的 cc 编译。
**验证:** `cargo test -p lumelog-app script::` 14 pass、`cargo test --workspace`（app 103/core 89）、`cargo clippy --workspace --all-targets -- -D warnings` 净、`npm test` 94 pass、`run build` 通过。

## 2026-10-25 — Plan 11 Roadmap BLF 报文源（feat/roadmap-blf）

**依据：** spec `docs/specs/2026-10-25-roadmap-blf-design.md`；计划 `docs/plans/2026-10-25-roadmap-blf.md`（Roadmap：Vector BLF，CAN/CAN-FD + zlib 容器，直接进 CanPanel，支持导出）。

### 终审修复波（CodeReview 全分支 1 Critical / 4 Important / 6 Minor）
**Files:** `blf.rs`（**C1**：classic `dlc` 钳到 8（未信任输入不得穿透到 `frame_json` 的 `data[..dlc]`）；**I2**：容器末尾 1–3 字节零填充不再当作残缺对象（EOF 误报截断）；**I3**：FD64 `ext_data_offset` 边界补回 `ext_len`（v1/v2 各差 16/24B，会误读扩展数据）；**I4**：`pending` 先出队再报错（容器内出错前的帧保留，spec §5）；**I5**：tail 上限只检查残留本身（不再误拒 >64MiB 合法容器）；**M1**：EOF/损坏 zlib → `source_format`；**M2**：channel u16→u8 钳位；**M3**：SYSTEMTIME 月日/闰秒校验；新增 9 测（dlc 钳位、86、v2 头、padding EOF、错误前帧、FD64 ext、无基准时间、坏 zlib、非法日期））；`dto.rs`（**C1 防御**：`frame_json` 的 `n = min(dlc, data.len())`）；spec 同步（FD64 边界、DIR 丢弃说明、`lineNumber`=帧序号）。
**Why:** Critical 使一个字节即可 panic 并让解析/导出 UI 永久卡死；I2/I3/I4 分别影响合法文件判定、FD64 数据正确性与“已解析帧丢失”；I5 与解压上限设计自相矛盾。
**验证:** `cargo test -p lumelog-can blf::` 19 pass + `cargo test -p lumelog-app` 90 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净。
**Parks（Minor，推迟）：** M4 容器仍整段解压/复制（已有 256MiB 上限，逐块流式留待后续）；M5 `BlfStats.error` 仍透出英文 `Display`（与文本源错误码体系统一留待后续）。
**定向复审补充（W1/S1/S2）：** I2/I3 的新测原本未真正越过边界（夹具恒 136B/valid=3 新旧同值）→ 改为 75B 对象带 3B 填充、valid=8 + 0xEE 扩展字节（修复前必红）；spec 补“容器末尾 <16B 全零=填充丢弃”规则；`open` 头部扩展截断也映射为 `source_format`。

### Task 1. core `lumelog-can/src/blf.rs`
**Files:** Create `crates/lumelog-can/src/blf.rs`（`BlfSource: FrameSource`；`LOGG` 头/`SYSTEMTIME`、`LOG_CONTAINER`（无压缩/zlib2、256MiB 上限）、v1/v2 对象、类型 1/86/100/101、跨容器 tail、`take_error`、Hinnant 日历、10µs/1ns）；Modify `lib.rs`（`pub mod blf` + `pub use BlfSource`）、`Cargo.toml`/`Cargo.lock`（`flate2 = "1"`）；spec 同步 FD64 data 偏移（payload@40，ext 边界 `ext-(16+ext_len+40)`）。
**Why:** 与项目自带 candump/ASC 解析一致、可行性可控；不依赖第三方 BLF crate。FD64 首版按参照实现 `cursor+40` 偏移会错 16B（测试捕获：数据从第 17 字节开始）→ 按真实 40B 结构改为 payload@40。
**验证:** `cargo test -p lumelog-can blf::` 10 pass（经典字段/时间 1.5s、ext/RTR/channel、FD100 dlc 9→12 + edl/brs/esi、FD64 64B、zlib 等价、跨容器一帧、10µs、未知对象 skipped、坏 magic/截断）+ `cargo clippy -p lumelog-can --all-targets -- -D warnings` 净。另加护栏：跨容器 partial tail > 64MiB 直接报错（不无界增长）。

### Task 2. 抽取 `export_frames`（行为不变）
**Files:** Modify `app/src-tauri/src/can_export.rs`（新增 `export_frames(src:&mut dyn FrameSource, ...)`，把表头/列/循环/统计原样搬移；`export_can` 仅建 `DocLines`+`build_frame_source` 后调用它）。
**Why:** BLF 无 `DocLines`，需与文本源共用帧→行/过滤/取消/统计逻辑；行为与签名不变。
**验证:** `cargo test -p lumelog-app can_export::` 20 pass（既有断言零改动）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 3. `can_parse_blf` + `can_export_blf_csv/jsonl`
**Files:** Modify `app/src-tauri/src/commands.rs`（三个命令 + `run_blf_export` 骨架；`BlfSource::open` 在 prologue 同步校验，坏文件直接 reject；解析复用 `parse_frames`，导出复用 `export_frames`；`take_error` 写 `BlfStatsJson.error` / 置 `errored`；取消复用 `can_cancel`）；`dto.rs`（`BlfStatsJson` + camelCase 测试）；`lib.rs`（注册）。
**Why:** spec §3；无源文档，BLF 帧仍走同一批处理/过滤/取消链路；大文件不整文件载入。
**验证:** `cargo test -p lumelog-app` 90 pass（+1 DTO）+ `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 4. 前端 IPC（BLF）
**Files:** Modify `app/frontend/src/ipc.ts`（`BlfStats`、`canParseBlf`、`exportBlfCsv/Jsonl`）；`ipc.test.ts`（+1 typeof）。
**Why:** spec §4；导出包装用 `path` 而非 `docId`，其余形状与现有 CAN 包装一致（双 Channel）。
**验证:** `npm test` 89 pass（ipc 15→16）+ `run build` tsc 通过。

### Task 5. `main.ts` BLF 视图
**Files:** Modify `app/frontend/src/main.ts`（工具栏“打开 BLF…”；`BLF_KEY=-2`+`blfView`；`openBlfFile/startBlfParse`（重置 store、同步过滤器、`canParseBlf` 流式推入、统计/错误 toast、停止复用 `can_cancel`）；`switchCanView`/`applyFilterFromInputs` 按 BLF 视图键；行激活提示无源文本；切标签退出 BLF；导出按视图走 `exportBlf*`，默认名取 BLF 基名；解析按钮文案随视图切换）。
**Why:** spec §4；复用 CanPanel/FrameStore/过滤器，不新增视图组件；坏文件 catch 后回到文档视图。
**验证:** `run build` tsc 通过 + `npm test` 89 pass（DOM 交互留 GUI 手工）。

## 2026-10-25 — Plan 10 Roadmap tail 跟随（feat/roadmap-tail-follow）

**依据：** spec `docs/specs/2026-10-25-roadmap-tail-follow-design.md`；计划 `docs/plans/2026-10-25-roadmap-tail-follow.md`（Roadmap：只读 tail 跟随 + 滚动截断，增量扩展索引）。

### 终审修复波（CodeReview 全分支 0 Critical / 4 Important / 5 Minor，全修）
**Files:** `file.rs`（**I1**：`try_append` 改用**路径新开探测句柄**读头/追加，成功后重绑 `self.file`（旧句柄在 rename 后指向旧 inode，导致更大新文件永远无法识别轮转）；新增 `try_append_rename_growth_is_rotated`；**I2**：`sig_len/head_len` 与实际 `head.len()` 取 min（stat 与读之间被截断不再 slice panic）；**M4**：按 `READ_CHUNK` 分块读+逐块 extend，避免巨型分配；预检 `new_size>u32::MAX`）；`line_index.rs`（**I3**：`extend` 先收集再一次性提交（失败不留部分偏移）+ 总大小越界拒绝）；`main.ts`（**I4**：chip 绑 owner docId、点击不再误操作活动文档；`syncStatus` 仅对活动文档刷 follow 并在切回 `on&&!paused` 时滚底；**M1**：追加后活动文档同步 `setReadonly/updateInspector`；**M2**：`tailSeenSize` 检测半行增长并 `invalidate/refresh`；**M3**：toast 改为按 docId 计数；关闭仅清自己的 chip/计数）；`listview.ts`（`scrollToBottom` 在 `clientHeight==0` 时返回，隐藏标签不再丢失跟随位置）；`statusbar.ts`（`clear()` 隐藏跟随按钮）；spec/plan 同步。
**Why:** rename 轮转漏检会让跟随静默停在旧文件；短读 panic、扩展失败留下不一致索引均属正确性缺陷；chip 跨标签误操作/隐藏标签被夹到顶部是明显 UX 错误。
**验证:** `cargo test --workspace`（app 89 / core 89，含 rename 新测）+ `cargo clippy --workspace --all-targets -- -D warnings` 净 + `npm test` 88 pass + `run build` tsc 通过。

### Task 1. core `LineIndex::extend`
**Files:** Modify `crates/lumelog-core/src/doc/line_index.rs`（`extend(appended,base_offset)`：只扫追加字节 `0x0A`，推 `base+i+1`；越界→`TooLarge`；更新 `file_size`）+ 5 单测。
**Why:** tail 追加不应重建全文；尾有/无 LF 两种旧状态均只需推 LF+1（不重复起点）。
**验证:** `cargo test -p lumelog-core line_index::` 14 pass（+5：尾有 LF/尾无 LF/空追加/多 LF/4GB 越界）+ `cargo clippy -p lumelog-core --all-targets -- -D warnings` 净。

### Task 2. core `DocFile::try_append` + 头签名
**Files:** Modify `crates/lumelog-core/src/doc/file.rs`（`head_sig`（FNV-1a64, 前 `min(512, old_size)`）；`AppendOutcome{NoChange,Appended{lines_added,total_lines},Rotated}`；`try_append`：缩小/头签名变→Rotated，同尺寸→NoChange，增长→`LineIndex::extend`+刷新 `size/mtime/head_sig`；`reload_after_save` 同步签名）+ 5 单测。
**Why:** tail 增量不重建全文；签名长度必须用**旧 size 的前缀**（否则增长过 512 后哈希窗口变化会误判轮转—— 首版测试即捕获）；`try_append` 刷新快照避免 B2 watcher 重复触发。
**验证:** `cargo test -p lumelog-core` 88 pass（+5：追加/无变化/截断/同大小改头/尾 LF）+ `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 3. `doc_tail_append` 命令 + DTO
**Files:** Modify `crates/lumelog-core/src/doc/file.rs`（修正：`head_sig` 窗口长度改为**打开时冻结的 `head_len`**——否则文件增长后哈希窗口变化会把正常追加误判为 Rotated；`reload_after_save` 同步 `head_len`）；`crates/lumelog-core/src/doc/mod.rs`（re-export `AppendOutcome`）；`app/src-tauri/src/dto.rs`（`TailAppendJson`）；`commands.rs`（可测 `tail_append` helper + `doc_tail_append` 命令 + `tail_tests` 2 例）；`lib.rs`（注册）。
**Why:** spec §3；`Rotated` 不修改索引、交给前端 `doc_reload`；`NoChange` 返回当前行数。
**验证:** `cargo test --workspace`（app 89 / core 88）+ `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 4. 前端 IPC 桥 + ListView 滚动辅助
**Files:** Modify `app/frontend/src/ipc.ts`（`TailAppend` 类型 + `docTailAppend`）；`listview.ts`（`isNearBottom` 纯函数、`scrollToBottom()/maxScrollTop()`、`ListViewOpts.onScrollTop` 在现有 `onScroll` 中回调）；Tests `ipc.test.ts`(+1)、`listview.test.ts`(+3)。
**Why:** spec §4。`onScrollTop` 从现有滚动路径回调（不另加监听，避免双触发）；`isNearBottom` 兼容不可滚动视图（max≤0→真）。
**验证:** `npm test` 86 pass（listview 10→13、ipc 14→15）+ `run build` tsc 通过。

### Task 5. 状态栏“跟随尾部”开关
**Files:** Modify `app/frontend/src/statusbar.ts`（`onFollow` 回调；`.status-follow` 按钮 + `setFollowAvailable/setFollow`（`active`/`aria-pressed`））；`styles.css`（`.status-follow/.active`、备用 `.follow-resume`）；Test `statusbar.test.ts`（+2）。
**Why:** spec §4—只读才显示；离开只读文档时自动清除 active；点击切换回调。
**验证:** `npm test` 88 pass（statusbar 2→4）+ `run build` tsc 通过。

### Task 6. `main.ts` 跟随状态 + 只读 `changed` 增量分流
**Files:** Modify `app/frontend/src/main.ts`（`followStates`/`followResumeEl`/提示节流；状态栏 `onFollow`；`syncStatus` 按编辑/只读切换按钮与状态；`setFollow`/`onListScroll`（`isNearBottom`）/`showFollowResume`/`hideFollowResume`；`handleReadonlyAppend`：`docTailAppend`→`rotated` 走 `reloadDocFromDisk`，否则更新 `totalLines/indexed`+`refresh`，follow&&!paused→`scrollToBottom`，≥1s 节流 toast；`reloadDocFromDisk` 只读分支按 follow 决定滚底/保位；`handleClose`/`mountDoc` 清理状态）。
**Why:** spec §4/§5。“回到底部”点击恢复跟随；滚动离底暂停；只读外的路径保持 B2 整档重载。
**验证:** `run build` tsc 通过 + `npm test` 88 pass（无回归；DOM 交互留 GUI 手工）。

## 2026-10-25 — Plan 9 M3 快捷键自定义（feat/m3-keymap）

**依据：** spec `docs/specs/2026-10-25-m3-keymap-design.md`；计划 `docs/plans/2026-10-25-m3-keymap.md`（M3 子项目 C：13 个动作可绑定、`config.toml` `[keymap]` 覆盖、录制模态）。

### 终审修复波（CodeReview 全分支 1 Critical / 3 Important / 5 Minor，全修）
**Files:** `keymap.ts`（**Critical**：`mergeBindings` 先 `normalizeAccel`+`validate`，非法覆盖不遮挡默认并记 `invalid`；`validate` 拒绝保留键 F3（任意修饰）与 Ctrl+F；`+`→`Plus`）；`keymap.test.ts`（+2 用例/改断言）；`commands.rs`（`read_config_text→Result`：NotFound=空、真实 IO 错误上浮，避免读失败后整文件覆写；`recovery_set_enabled` 复用同一读写 helper；`keymap_*` 用 `?`）；`keymap-ui.ts`（`onSet/onResetAll` 返回新合并表、模态据此整表重绘；`#app` inert；初始聚焦关闭钮；说明补系统键/布局）；`keymap-ui.test.ts`（mock 返回合并表）；`main.ts`（`applyKeymapState` 上报 `invalid`；侧栏“快捷键设置…”免快捷键入口；移除查询按钮硬编码键提示；`applyKeymap/resetKeymapAll` 返回合并表）；`styles.css`（`.side-action`）；spec/plan 同步。
**Why:** Critical 为真安全缺口（手写 `save="S"` 会变成全局裸字母键、劫持输入并反复保存）；Important：F3/Ctrl+F 明明不可自定义却可录制/持久化、读配置失败会静默覆写整个 `config.toml`、`palette` 解绑后设置面板不可达；Minor：无焦点陷阱、`Ctrl++` 不可解析、模态快照与 live 合并不一致、工具栏键提示陈旧、缺系统键提示。
**验证:** `cargo test -p lumelog-app` 87 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净 + `npm test` 82 pass（keymap 9→11）+ `npm run build` tsc 通过（45 modules）。
**定向复审：** 9/9 ADDRESSED、无新增破坏；追加：`invalid` toast 区分“已用默认/已置未绑定”（无默认的 7 个动作不再误报）。

### Task 1. 后端 `keymap` 存储层
**Files:** Create `app/src-tauri/src/keymap.rs`（`read_keymap`/`set_binding`/`reset_keymap` + 7 单测）；Modify `app/src-tauri/src/lib.rs`（`mod keymap;`）。模块级 `#![allow(dead_code)]`（Task2 接入后剥）。
**Why:** spec §4—`[keymap]` 表：缺失→默认、`""`→解绑、`set_binding(None)`→删键回默认；保留 `recovery_enabled`/`theme` 等其它配置键；删空表时不写空 `[keymap]`；坏 TOML 重建。
**验证:** `cargo test -p lumelog-app keymap::` 7 pass（缺表/非串跳过、保留其它键、None 删键、删空表、`""` 解绑、reset 只删表、坏文本重建）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 2. `keymap_get` / `keymap_set` / `keymap_reset` 命令
**Files:** Modify `app/src-tauri/src/commands.rs`（`read_config_text`/`write_config_text` + 3 命令；读写走现有 `config_path`）；Modify `app/src-tauri/src/lib.rs`（注册）；`keymap.rs`（剥模块 allow）。
**Why:** spec §4；`keymap_set` 的 `accel=None` 回默认、`Some("")` 解绑；写盘前 `create_dir_all`；失败上浮 `io`。
**验证:** `cargo test -p lumelog-app` 87 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 3. 前端 `keymap.ts`（规范/校验/合并）
**Files:** Create `app/frontend/src/keymap.ts`（`ActionId`/`ACTION_IDS`/`DEFAULT_BINDINGS`、`eventToAccel`、`parseAccel`/`formatAccel`/`normalizeAccel`、`validate`、`mergeBindings`、`conflictOf`）；Create `keymap.test.ts`（9 测）。
**Why:** spec §3。Ctrl 同时匹配 Ctrl/Cmd；validate 要求 Ctrl/Alt 或 F1–F24（防劫持）；合并先默认后覆盖（`""`=解绑），冲突先到先得并记录，未知 id 上报。
**验证:** `npm test` 77 pass（+9）+ `run build` tsc 通过（43 modules）。

### Task 4. 前端 IPC 桥（快捷键）
**Files:** Modify `app/frontend/src/ipc.ts`（`keymapGet/keymapSet/keymapReset`）；Modify `ipc.test.ts`（+1 typeof）。
**Why:** spec §4；`keymapSet` 的 `accel=null` 回默认、`""` 解绑。
**验证:** `npm test` 78 pass（ipc 13→14）+ `run build` tsc 通过。

### Task 5. `ACTION_DEFS` 单一动作源 + 表驱动分发
**Files:** Modify `app/frontend/src/main.ts`（`ACTION_DEFS`（13 项）+ `actionTitle`；`initKeymap`/`applyKeymapState`（冲突/未知 id toast）；`buildCommands` 由 `ACTION_DEFS` 生成 + 保留 theme-*；`registerShortcuts` 改 `eventToAccel→accelToAction→run`，F3 分支保持首位，模态打开时短路；启动 `initKeymap`）。
**Why:** spec §2/§3/§5—面板、分发、模态共用一套动作；旧硬编码 Ctrl+S/O/G/Shift+F/Shift+P 全部退出，生效键完全由 `keymapState` 决定。
**验证:** `run build`（tsc 通过，44 modules）+ `npm test` 78 pass。

### Task 6. 录制模态 + 样式 + 接线
**Files:** Create `app/frontend/src/keymap-ui.ts`（`openKeymapModal`：行=标题/当前键/录制/清除/默认 + 错误行；录制独占 keydown（capture）+ Esc 取消 + `validate` + `conflictOf` 红字；“全部恢复默认”后本地重置为空默认集）；Create `keymap-ui.test.ts`（2 个 jsdom 用例：冲突拒/裸键拒/Alt+K 落盘/close changed、Esc 取消不落盘）；Modify `main.ts`（`applyKeymap`/`resetKeymapAll`/`openKeymapSettings` + 面板“快捷键设置…”）；`styles.css`（`.keymap-*`）。
**Why:** spec §5。写盘失败时模态保留旧值并提示；模态打开时 `registerShortcuts` 短路；确认关闭回调只负责落 `keymapModalOpen`。
**验证:** `npm test` 80 pass（13 文件，+2）+ `run build` tsc 通过（45 modules）。

## 2026-10-25 — 测试基建：jsdom DOM 单测

**Files:** `app/frontend/package.json` + `package-lock.json`（`jsdom` devDependency）；`app/frontend/src/tabs.test.ts`（`@vitest-environment jsdom`；新增 2 个真实 DOM 用例：黄点 `setExternalFlag` 与脏点独立切换）；Create `app/frontend/src/statusbar.test.ts`（2 个用例：`setWatch` ok 隐藏 / degraded 显示文案+tooltip 并可清除）。
**Why:** 上轮 B2 因项目无 jsdom，`TabStrip`/`Statusbar` 只能抽纯函数测（`externalDot`）或留 GUI 手测；补 jsdom 后用 per-file 指令启用，不影响其余 64 个 headless 用例。
**验证:** `npm --prefix app/frontend test` 68 pass（11 文件；tabs 2→4、statusbar +2）+ `run build`（tsc 通过，43 modules）。

## 2026-10-25 — Plan 8 M3 外部修改监听（feat/m3-external-watch）

**依据：** spec `docs/specs/2026-10-25-m3-external-watch-design.md`；计划 `docs/plans/2026-10-25-m3-external-watch.md`（M3 子项目 B2：已打开文档的外部修改持续监听 + 干净自动重载 + 脏黄点/三选）。

### 终审修复波（CodeReview 全分支 1 Critical / 4 Important / 3 Minor，全修）
**Files:** `watch.rs`（**Critical**：watcher 移入独立 `Mutex`，`notify` 的 watch/unwatch 不再持 `shared` 锁调用（回调会抢它，可能死锁）；`registry.paths` 改 `HashMap<PathBuf,u32>`（同文件多标签/关闭重开不再丢监听）；`status()` 公开）；`state.rs`（`DocSnapshot` 别名；`snapshot_for_path` 在 doc 锁内成对读 `disk_meta`+当前磁盘，并按 `canonicalize` 比较路径）；`commands.rs`（**Save-As 后端重绑 `DocFile` 到新路径**（path/index/cache/快照/冲突基线跟随）并在**释放 doc 锁后** `retarget(old,new)`；`watch_retarget` 改为 `watch_status`）；`lib.rs`；`ipc.ts`/`ipc.test`（`watchStatus`）；`main.ts`（`confirmExternalConflict(detail)` 保留 `err.zh` 知情细节；重载 `await` 后重校验关闭/重开与“途中输入”保护；`handleClose→Promise<boolean>`，模式切换不二次确认也不误开新标签；打开后刷新 watch 状态）；docs spec/plan 同步。
**Why:** Critical 为真实死锁窗口（持锁调 notify 等事件线程 ack，而事件线程在 handler 里等同一把锁）；Important：Save-As 后监听静默失效（doc 仍绑旧 path→新路径事件永无匹配、`doc_reload` 也会重载旧文件）、三选丢失冲突 detail、同路径多文档丢监听、重载后不校验状态（可覆盖途中输入/向已关标签写状态）。Minor：快照与 stat 的 TOCTOU 误报、启动后降级不上报、模式切换依赖已捕获引用。
**验证:** `cargo test -p lumelog-app` 80 pass（+`registry_same_path_keeps_watch_until_last_close`）+ `cargo clippy --workspace --all-targets -- -D warnings` 净 + `npm test` 64 pass + `npm run build` tsc 通过。
**定向复审：** 8/8 ADDRESSED、无阻塞新问题；追加修：模式切换关标签后 `watchPending.delete`（防重放到死 docId）、`afterSave` 后 `refreshWatchStatus`（Save-As 新目录降级及时上报）、spec §5 同步 `watchStatus`。

### Task 1. `watch` 纯逻辑
**Files:** Create `app/src-tauri/src/watch.rs`（`ExternalKind{Changed,Removed}` lowercase serde；`classify(disk,snapshot)`；`WatchRegistry{dirs refcount,paths}` + `watch/unwatch/tracked`；`ExternalEvent{path,kind}` + 7 单测）；Modify `app/src-tauri/src/lib.rs`（`mod watch;`）。模块级 `#![allow(dead_code)]`（Task2 接入后剥）。
**Why:** spec §2—`classify` 用式样：`disk==snapshot`→None（自我写入/无变化）、`None`→Removed、否则 Changed；监视单位=父目录，同目录 refcount 归零才停；裸文件名退化为 `.`。
**验证:** `cargo test -p lumelog-app watch::` 7 pass（等值忽略/size或mtime变/缺失/同目录只 watch 一次/refcount 归零/相对名 `.`/事件 lowercase 往返）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 2. `AppWatch` 运行态 + 订阅 + 文档生命周期挂钩
**Files:** Modify `app/src-tauri/src/watch.rs`（`WatchStatus`、`Inner`、`AppWatch{start/watch_path/unwatch_path/retarget}`、`debounce_loop`（Condvar 500ms 静默窗）、`WatchRegistry::dirs`）；Modify `state.rs`（`snapshot_for_path` + 测）；Modify `commands.rs`（`watch_subscribe`/`watch_retarget`；`doc_open` 成功后 `watch_path`、`doc_close` 前 `unwatch_path`）；Modify `lib.rs`（manage `AppWatch` + 注册 2 命令）；Modify `Cargo.toml`（`notify="6"`）+ `Cargo.lock`。
**Why:** spec §2。watcher 回调只入 `pending` 并唤醒；去抖线程等首个事件后持续等到 500ms 无新事件才 drain；drain 时按 `snapshot_for_path` 逐 doc `classify`（Removed 优先、每 path 只发一条）；watcher 创建失败→`degraded` 不阻塞。
**验证:** `cargo test -p lumelog-app` 76 pass（+2：`snapshot_for_path` 同 path 两 doc/未知 path；`watch_status_starts_not_degraded`）+ `cargo clippy --workspace --all-targets -- -D warnings` 净（notify 6.1.1 编译通过）。

### Task 3. `AppDocs::replace` + `doc_reload`
**Files:** Modify `app/src-tauri/src/state.rs`（`replace(id,doc)` 锁同一 Arc 原地赋值 + 测）；Modify `commands.rs`（可测 `reload_doc(state,doc_id)` + `#[tauri::command] doc_reload`；重开前 `validate_open_path`，超限→`too_large`；返回新 `DocInfo`（同 id，mode 可能变）+完成 tick；+ `reload_tests` 2 例）；Modify `lib.rs`（注册）。
**Why:** spec §3。原地替换保 Arc 一致（`DocLines`/导出持有的 Arc 立即看到新文件）；`DocFile::open` 重建索引/刷新 `(size,mtime)`。测试夹具采用无尾换行（核心行语义：尾 `\n` 会多一空行）。
**验证:** `cargo test -p lumelog-app` 79 pass（+3：replace 同 id 新 doc/未知 id；reload 外部改 3→5 行、`chunk_at` 新文本、`disk_meta` 刷新；未知 doc 码）；`cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 4. 前端基础件（ipc / tabs / listview / statusbar）
**Files:** Modify `ipc.ts`（`ExternalEvent`/`WatchStatus`/`watchSubscribe`/`docReload`/`watchRetarget`）；`tabs.ts`（`TabEntry.ext` `.ext-dot` + 可测纯函数 `externalDot(on)` + `setExternalFlag`）；`listview.ts`（`scrollTop()`/`setScrollTop()`）；`statusbar.ts`（`.status-watch` + `setWatch("ok"|"degraded", reason)`）；`styles.css`（`.ext-dot/.warn`、`.status-watch`）；Tests `ipc.test.ts`(+1)、Create `tabs.test.ts`(2)。
**Why:** spec §3。项目无 jsdom，TabStrip 本体不可无依赖测——把黄点表现抽为 `externalDot` 纯函数单测；`.ext-dot` 独立于 `.dirty-dot`。
**验证:** `npm test` 64 pass（10 文件，+3）+ `run build` tsc 通过（43 modules）。

### Task 5. `main.ts` 外部事件分发
**Files:** Modify `app/frontend/src/main.ts`（`initWatch` 启动订阅 + 降级写状态栏；`handleExternalEvent` 按 path 反查 docId：removed→黄点+toast；脏→黄点+`.ext-bar`（重新加载/保留）；干净→`reloadDocFromDisk`；单飞 `watchReloading`/`watchPending`；编辑保 scroll/选区、只读保 scroll + 清查询；跨 200MB 模式变化→关+重开；重载失败→黄点保留；`afterSave`/`handleClose` 清黄点与提示条）。
**Why:** spec §3。重载在 `docReload` 后重新拼装状态（encoding/eol/totalLines/invalid）并 `markSaved`；黄点/提示条在保存/关闭/成功重载时清除。
**验证:** `run build`（tsc 通过）+ `npm test` 64 pass（无回归；DOM 路径留 GUI 手工）。

### Task 6. 保存三选模态 + Save-As 监听迁移 + 样式
**Files:** Modify `app/frontend/src/main.ts`（`confirmExternalConflict()` 自绘三键模态（重加载/强制覆盖/取消，Esc/遮罩=取消）；`saveActive` 冲突分支：`conflict`→三选（reload 走 `reloadDocFromDisk` 丢弃本地修改）、`unmappable` 仍两键 `ask`；`afterSave` 当 `target!==prevPath` 时 `watchRetarget`）；`styles.css`（`.ext-bar`/`.ext-label` + `.modal-3way*`）。
**Why:** spec §3。仅 `conflict` 走三选，避免与编码门语义混淆；Save-As 后监听迁移到新路径（后端从 doc 取旧 path）。
**验证:** `run build`（tsc 通过，43 modules）+ `npm test` 64 pass。

## 2026-10-25 — Plan 7 M3 崩溃恢复（feat/m3-crash-recovery）

**依据：** spec `docs/specs/2026-10-25-m3-crash-recovery-design.md`；计划 `docs/plans/2026-10-25-m3-crash-recovery.md`（M3 子项目 B1：脏文档影子文件 + 启动单列表横幅恢复 + 最小配置开关 `recovery_enabled`）。

### 终审修复波（CodeReview 全分支 1 Critical / 3 Important / 4 Minor，全修）
**Files:** `app/frontend/src/editor.ts`（`OpenEditableOpts.onEdit` 每次改动触发；`onDirty` 仍只在首次脏转变；`replaceText`/`setEol` 也触发 `onEdit`）；`app/frontend/src/main.ts`（`onEdit→scheduleRecovery`（onDirty 每次脏周期只触发一次，是 Critical 根因）；`openPath→Promise<boolean>`；恢复时**复用已开同路径标签**（`tabs.list/activate`，避免双 docId 抢同一影子/文件）；`cancelRecovery` 同步移除横幅行；恢复后 `updateInspector`+`refreshSidebar`；`recoveryWrites` 按 path 串行化在飞写入，discard 等写入落地后再删（防“返回的旧影复活”）；`recovery_discard` 错误上浮）；`app/src-tauri/src/recovery.rs`（`discard_shadow→io::Result`，仅 NotFound 视为成功）；`app/src-tauri/src/commands.rs`（`recovery_discard` 映射 `io`）；`app/frontend/src/styles.css`（`#toast` `z-index:70` 高于横幅 60）。
**Why:** Critical（影子在整个脏周期只写一次→长时间编辑后崩溃只恢复首次输入后 2s 快照）使验收项失效；Important 三项（缺失文件提示不可达、恢复重复开页、保存/关闭后横幅僵尸行）均为真实数据/UX 缺口；Minor 四项（信息栏不刷新、写入/discard 竞争、删除错误被吞、toast 被横幅遮）顺手修正。
**验证:** `cargo test -p lumelog-app recovery::` 11 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净 + `npm run build`（tsc，43 modules）+ `npm test` 61 pass。GUI 必测：长时间编辑→停输 2s→再编辑→`kill -9`→恢复为最后文本。

### Task 1. `recovery` 影子存储层
**Files:** Create `app/src-tauri/src/recovery.rs`（`RecoveryMeta/RecoveryEntry/RecoveryContent` camelCase；`fnv1a64`/`doc_key`/`shadow_path`/`write_shadow`/`read_shadow`/`list_shadows`/`discard_shadow` + 7 单测）；Modify `app/src-tauri/src/lib.rs`（`mod recovery;`）。模块级 `#![allow(dead_code)]`（Task2 接入后剥）。
**Why:** spec §2—影子 `<doc_key>.lumelog` = 首行 JSON meta + `\n` + 原始 UTF-8 文本（大文本免 JSON 转义）；`doc_key=FNV-1a64(path)` 16hex（零依赖、稳定）；写入 `.lumelogtmp`→rename；list 按 `saved_at_ms` 降序、坏文件跳过、缺失目录→空。
**验证:** `cargo test -p lumelog-app recovery::` 7 pass（FNV 标准向量、key 稳定/区分、meta+文本往返含中文/emoji/CRLF、同 key 覆盖、排序+坏文件跳过、缺目录、discard 幂等）；`cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 2. 存储命令 + 最小配置命令 + 注册
**Files:** Modify `app/src-tauri/src/recovery.rs`（`read_recovery_enabled`/`set_recovery_enabled` + 4 配置/DTO 测，剥模块 allow）；Modify `app/src-tauri/src/commands.rs`（`recovery_root/dir/config_path/recovery_enabled_now` + 6 命令 `recovery_save/list/load/discard/enabled/set_enabled`；禁用时 save no-op、list 空）；Modify `app/src-tauri/Cargo.toml`（`toml="0.8"`）+ `Cargo.lock`；Modify `lib.rs`（注册 6 命令）。
**Why:** spec §3/§3.1。命令全以 `path` 为键（后端推 FNV）；config.toml 顶层 `recovery_enabled`（缺失/坏→true），set 保留其它键；写回前 `create_dir_all`；`recovery_save` 防御式再查开关。
**验证:** `cargo test -p lumelog-app` 67 pass（recovery 11：含 4 配置/DTO）+ `cargo clippy --workspace --all-targets -- -D warnings` 净（新 `toml` 依赖编译通过；Cargo.lock 随本次提交）。

### Task 3. 前端 `recovery.ts`（Debouncer + 格式化）
**Files:** Create `app/frontend/src/recovery.ts`（`Debouncer`（按 docId 合并/取消/cancelAll）+ `humanSize` + `formatSavedAt`）；Create `recovery.test.ts`（5 测）。
**Why:** spec §4/§6。把 2s 节流的时序与展示格式抽为 DOM 无关纯模块，便于 fake-timers 单测；不依赖 locale 避免平台差异。
**验证:** `npm --prefix app/frontend test` 60 pass（+5：合并只写一次/取消/多 id 隔离+cancelAll/humanSize 边界/时间格式形状）+ `run build` tsc 通过。

### Task 4. 前端 IPC 桥（崩溃恢复）
**Files:** Modify `app/frontend/src/ipc.ts`（`RecoveryEntry`/`RecoveryContent` 类型 + `recoverySave/recoveryList/recoveryLoad/recoveryDiscard/recoveryEnabled/recoverySetEnabled`）；Modify `ipc.test.ts`（+1 typeof）。
**Why:** spec §3/§3.1；`RecoveryEntry` 字段与 Rust camelCase 对齐（`savedAtMs/sizeBytes`）；均以 `path` 为键。
**验证:** `npm test` 61 pass（ipc 11→12）+ `run build` tsc 通过；Rust 未改。

### Task 5. `main.ts` 排影 + 清理
**Files:** Modify `app/frontend/src/main.ts`（导入 `Debouncer`/`recoverySave`/`recoveryDiscard`；`recoveryOn`/`recoveryDebounce`；`fireRecovery`/`scheduleRecovery`/`cancelRecovery`；`mountDoc.onDirty` 追加 schedule；`saveActive` 记 `prevPath` 并传给 `afterSave`（签名 +1 参）；`afterSave` 末尾清旧/新 path 影子；`handleClose` 关前清影；`changeEncoding` 丢弃重载后清影）。
**Why:** spec §4。2s 节流只在“编辑模式 + 脏 + 开关开”时排影；保存（含 Save-As 前后两 path）/关闭/换编码丢弃后删影 + 清定时器；写影失败 best-effort 不拖断编辑。
**验证:** `npm --prefix app/frontend run build`（tsc 通过，43 modules）+ `npm test` 61 pass（无回归；时序逻辑已由 T3 单测覆盖）。

### Task 6. 侧栏勾选框 + 启动横幅 + 恢复/丢弃
**Files:** Modify `app/frontend/src/main.ts`（侧栏 `.side-option` “崩溃恢复”复选框；`initRecovery` 读配置回填 + 拉起 `checkRecovery`；`onRecoveryToggle` 写配置/回滚/禁用时 `cancelAll`+收横幅；`renderRecoveryBanner`/`discardRecovery`/`discardAllRecovery`/`restoreRecovery`（`openPath`→`recoveryLoad`→`replaceText`→套 encoding/eol→仍脏→删旧影）；启动尾部 `void initRecovery();`）；Modify `styles.css`（`.side-option` + `.recovery-banner/head/list/row/label/foot`，用现有主题 token）。
**Why:** spec §4。横幅不阻塞启动；原文件缺失时提示且**不删条目**（仅显式丢弃）；禁止/开启与 config.toml 双向一致，写失败回滚勾选。
**验证:** `run build`（tsc 通过）+ `npm test` 61 pass；GUI 手工清单留 Task 7。

## 2026-10-25 — Plan 6 M3 CAN 报文导出（feat/m3-can-export）

**依据：** spec `docs/specs/2026-10-25-m3-can-export-design.md`；计划 `docs/plans/2026-10-25-m3-can-export.md`（M3 子项目 A 增量3：后端重解码流式导出 CAN 报文为宽表 CSV + JSONL）。

### 终审修复波（CodeReview 全分支 0 Critical / 0 Important / 1 Minor）
**Files:** Modify `app/src-tauri/src/can_export.rs`（CSV meta 第 4 列由十进制 `canId` 改为 `canIdHex`（`fj.can_id_hex`）以对齐 spec §2 与面板显示；同步 5 个测试断言）。
**Why:** 终审唯一发现：实现与 spec §2 规定的列不一致。采纳“代码跟 spec”（面板一贯 hex）。**验证:** `cargo test -p lumelog-app can_export::` 20 pass + `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 1. `can_export::frame_matches`（canfilter.ts 端口）
**Files:** Create `app/src-tauri/src/can_export.rs`（`frame_matches(fj:&FrameJson,f:&CanFilterJson)->bool` + 8 单测）；Modify `app/src-tauri/src/dto.rs`（`SignalOp`{Gt,Lt,Ge,Le,Eq → ">"/"<"/">="/"<="/"=="} + `CanFilterJson` camelCase/`default`/`skip_serializing_if`）；Modify `app/src-tauri/src/lib.rs`（`mod can_export;`）。`frame_matches`/`CanFilterJson`/`SignalOp` 暂加带注释 `#[allow(dead_code)]`（Task4/5 接入后剥）。
**Why:** 后端导出需与 CanPanel 视图过滤一致。作用在 `FrameJson`（与前端同结构）逐字端口 `canfilter.ts`；ID 闭/反向不匹配、msgName 精确、信号三要素齐才判、`Eq` 用 `to_bits()` 复现 JS `Object.is`（NaN==NaN 真、+0≠-0）、时间窗缺 ts 不匹配。
**验证:** `cargo test -p lumelog-app can_export::` 8 pass（逐字对应 canfilter.test.ts：空过滤/ID 闭/反向/msgName/信号 gt+缺失/`Eq` Object.is 语义/时间窗/AND 组合）；`cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 2. 抽 `build_frame_source` + `validate_source_kind_regex`（can_parse 行为不变重构）
**Files:** Modify `app/src-tauri/src/commands.rs`（新 `pub(crate) validate_source_kind_regex(kind,regex)`（candump/asc OK；regex 缺 pattern→`bad_regex`、非法正则经一次性 `RegexSource::new(empty)`→`from_can`；未知 kind→`source_format`）+ `build_frame_source(kind,regex,lines)`（worker 用，返回 `Box<dyn FrameSource>`）；`can_parse` 两段内联逻辑改调用两函数（行为逐字一致）；+`source_validation_tests` 5 例）。
**Why:** spec §5—导出命令与 `can_parse` 共用同一源解析/校验，不第三套。`validate` 供 prologue、`build_frame_source` 供 worker，regex 编译校验集中在 validate（worker `expect("validated")`）。
**验证:** `cargo test -p lumelog-app` 43 pass（新 5：candump/asc 通过、缺 pattern、合法正则、非法正则拒、未知 kind；`can_parse` 相关测不回归）+ `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 3. CSV 宽表 / JSONL 纯文本构造器
**Files:** Modify `app/src-tauri/src/can_export.rs`（新 `qualified_columns`/`db_signal_columns`/`csv_header`/`csv_row`/`jsonl_line` + `csv_escape`/`fmt_physical`/`fmt_ts` 私有 + `CSV_META` 常量）+ 8 测）。
**Why:** spec §2/§3。列名 `消息名.信号名` 去重 `#n`；meta 7 列固定 + DBC 信号列；物理值 `{:.6}` 裁尾零（非有限→空）；CSV 仅在含 `,"\r\n` 时加引号；jsonl 复用 `serde_json::to_string`（FrameJson 本就 camelCase）。纯文本构造与文件 I/O 解耦→无盘可测。
**验证:** `cargo test -p lumelog-app can_export::` 16 pass（+8：列限定去重、db_signal_columns、header meta/含信号、row 填列与空列、转义、无 msg、浮点稳定/非有限、jsonl 合法回读）；`cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 4. `export_can` 扫描核
**Files:** Modify `app/src-tauri/src/can_export.rs`（新 `CanExportStats{written,scanned,cancelled,errored}` + `export_can<W:Write>(doc_arc,dbc,kind,regex,filter,writer,on_progress,cancel,wide)`：`DocLines::new`+`build_frame_source`→逐帧`next_frame`→`frame_json`→`frame_matches`→CSV/JSONL 行；读写错→`errored`、每帧判 `cancel`）+ 4 测；Modify `app/src-tauri/src/commands.rs`（`DocLines::new` 改 `pub(crate)` 供 can_export 复用）。
**Why:** spec §5。**优于计划草拟**：`export_can` 走 `DocLines` 按块加解锁（同 `can_parse`/`filter_run`），不持整轮 doc 锁→并发 `doc_chunk` 不饥死；`frame_json` 解码一次兼供过滤与取值。build/write 错吸收为 `errored`不返 Result。
**验证:** `cargo test -p lumelog-app can_export::` 20 pass（+4：jsonl 有数帧、CSV header+id 过滤、DBC 信号列、取消早停且 written<total）；`cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 5. `export_csv` / `export_jsonl` / `export_can_cancel` 命令
**Files:** Modify `app/src-tauri/src/commands.rs`（新 `run_can_export` 共享体 + `#[tauri::command] export_csv`/`export_jsonl`（wide 真/假）+ `export_can_cancel`；导入 `export_can`）；Modify `dto.rs`（`CanExportProgress`、`CanExportStatsJson`+`From<can_export::CanExportStats>`）；Modify `lib.rs`（注册三命令）；剥 `export_can` 的 `#[allow(dead_code)]`；+1 DTO 测。
**Why:** spec §5。prologue 同步校验源 + `File::create`（坏路径→`io` 不建 worker）→ `spawn_blocking` 跑 `export_can`、`on_progress` 节流、`on_done` 必发、`end_search`；早返 req_id。两命令共用 `run_can_export`。
**验证:** `cargo test -p lumelog-app` 56 pass（+`can_export_stats_json_from_and_camel_case`）+ `cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 6. 前端 IPC 桥（CAN 导出）
**Files:** Modify `app/frontend/src/ipc.ts`（`import type CanFilter`；新 `CanExportStats` 类型 + 内部 `exportCan` 共用体 + `exportCsv`/`exportJsonl`（Omit cmd）/`exportCanCancel`）；Modify `app/frontend/src/ipc.test.ts`（+`can_export_wrappers_are_functions`）。
**Why:** spec §4.2；两导出共用一个包装只发不同 cmd 名。filter 直接用 canfilter 的 `CanFilter`（与 Rust `CanFilterJson` camelCase 对齐）。
**验证:** `npm --prefix app/frontend test` 55 pass（ipc 10→11）+ `run build`（tsc 通过）；Rust 未改。

### Task 7. dock 导出按钮 + main.ts 流程
**Files:** Modify `app/frontend/src/main.ts`（CAN 态加 `canExportBusy/canExportReqId/canExportStatus/canExportCancelBtn/canExportBtns`；dock 工具条加“导出 CSV”/“导出 JSONL”/“取消导出”（隐）+ 状态 span；新 `onCanExport(format)`（save 对话框默认 `<docname>.csv/.jsonl`，取 dock `sourceKind/regex/lastDbcId/canFilters.get(docId)` → `exportCsv/exportJsonl`，进度写状态、`onDone` 区分 cancelled/errored/成功 toast）/`cancelCanExport`/`updateCanExportUi`；`parseActive` 守卫改为 `canParsing||canExportBusy` 共享单飞）。
**Why:** spec §4.1。导出与解析共用 dock 单任务互斥；filter 取当前面板组装值（与显示一致）。（tsc 修：`onDone` 参数标注 `CanExportStats`，对象字面量无上下文推导）
**验证:** `npm --prefix app/frontend run build`（tsc 通过，42 modules）+ `npm test` 55 pass；无新单测（DOM/IPC 密集，GUI 手工留 Task 8）。

## 2026-10-25 — Plan 5 M3 时序筛选 + 导出（feat/m3-timeline-export）

**依据：** spec `docs/specs/2026-10-25-m3-timeline-export-design.md`；计划 `docs/plans/2026-10-25-m3-timeline-export.md`（M3 子项目 A 增量2：只读日志时序筛选 + export_lines）。

### 终审修复波（CodeReview 全分支 0 Critical / 0 Important / 3 Minor）
**Files:** Modify `app/frontend/src/main.ts`（M1：导出默认名改为 `<docname>.filtered.log`（取 `docStates` 的 `basenameOf(path)`）；M2：`doExport` 开头加“仅只读视图”守卫（`lists.has`），避免对可编辑文档导出磁盘未保存旧字节）；Modify `app/src-tauri/src/commands.rs`（M3：导出 worker 注释改准——导出按整任务持 doc 锁，与 `filter_run`/`can_parse` 按块加解锁不同；`export_cancel` 走独立注册表仍即时、无死锁）。
**Why:** 终审仅余 3 项非阻塞 Minor，均低代价且 M2 是真实一致性缺口→一并修。**验证:** `npm test` 54 pass + `run build`（tsc）+ `cargo clippy --workspace --all-targets -- -D warnings` 净（本轮仅改 main.ts 逻辑 + 注释，Rust 行为无变）。

### Task 1. lumelog-core 行过滤器加时序 + 行号轴
**Files:** Modify `Cargo.toml`（root `[workspace.dependencies]` 加 `chrono = { version="0.4", default-features=false, features=["std"] }`）；Modify `crates/lumelog-core/Cargo.toml`（`chrono = { workspace = true }`）；Modify `crates/lumelog-core/src/filter.rs`（新 `TimeFormat{Iso,EpochSec,EpochMillis}`/`TimeSpec`/`parse_time_ms`；`LineFilter` 加 `time`/`line_range` 字段，`build` 新增 `time: Option<TimeSpec>, line_range: Option<(u64,u64)>` 参（校验抽取正则含命名组 `ts`、预解 from/to，非法→`InvalidPattern`），`keep(line_no,text)` 新增 `line_no` 参并依次过行号区间→文本 AND→时间窗（无 ts 行按 `keep_when_no_ts`））；Modify `app/src-tauri/src/commands.rs`（新签名涟漪：`scan_filter` 的 `lf.keep(line1-1,&text)`、`filter_run` 与测试 `make_filter` 的 `build(.., None, None)`）。
**Why:** spec §2/§3.1。行号与时间是两条独立 AND 轴；时间解析用内置格式集（chrono，无偏移当 UTC，支持 RFC3339/小数秒）；无抽取正则时行号区间为退化路径。**涟漪决策：**因 `build`/`keep` 签名变更，本 Task 一并适配 `commands.rs` 现有调用（传 `None,None` 保持旧行为），保证本提交 workspace 仍可编译（时间轴/行号轴的 **功能接入** 属 Task 4）。
**验证:** `cargo test -p lumelog-core filter::` 16 pass（旧 10 适配新签名 + 新 6：行号区间/ISO 窗/epoch 秒/无 ts 保留标记/缺 ts 组报错/ISO 偏移与小数秒）；`cargo test -p lumelog-app` 23 pass（旧 filter_run 测试适配）；`cargo clippy --workspace --all-targets -- -D warnings` 净。

### Task 2. lumelog-core::export 原始字节导出
**Files:** Create `crates/lumelog-core/src/export.rs`；Modify `crates/lumelog-core/src/lib.rs`（`pub mod export;`）。
**Why:** spec §3.2。`export::run<W:Write>(doc, matcher, writer, on_progress, cancel) -> ExportStats{written,scanned,cancelled,errored}` 逐块 `chunk_at`（`chunk_lines` 去尾换行与 filter_run 的 `DocLines` 同法，保证 keep 判定与屏显一致）→保留行用 `line_byte_range`+`read_window` 取**原始字节**写 `writer`（不转码，精确保编码/行尾）；每行判 cancel、每块报进度；读/写错吸收为 `errored` 不返回 Result（落 core 因 `line_byte_range/read_window` 是 `pub(crate)`）。
**验证:** `cargo test -p lumelog-core export::` 4 pass（字节一致/含选中行“bb\n”、CRLF+无尾换行全量逐字节、GBK 多字节原样拷贝、取消早停且输出为前缀）；`cargo clippy -p lumelog-core --all-targets -- -D warnings` 净。

### Task 3. 时序/导出 DTO（`lumelog-app`）
**Files:** Modify `app/src-tauri/src/dto.rs`（新增 `TimeSpecJson{extractPattern,format,from?,to?,keepWhenNoTs}`(+`to_core`→`TimeSpec`，未知格式→Iso)、`ExportProgress{written}`、`ExportStatsJson{written,scanned,cancelled,errored}`(+`From<core ExportStats>`)；均 camelCase；+`timeline_export_dto_tests`）。未接入命令前加带注释 `#[allow(dead_code)]`（Task 4/5 剥）。
**Why:** spec §3.3，为 `filter_run`/`export_lines` 定契约；时间轴编译错误由 core `LineFilter::build`→`IpcError::from_core`→`invalid_pattern`（无新错误码）。
**验证:** `cargo test -p lumelog-app` 26 pass（新 3：格式映射/驼峰导出/From core）+ `cargo clippy -p lumelog-app --all-targets -- -D warnings` 净。

### Task 4. `filter_run` 接时间轴/行号轴
**Files:** Modify `app/src-tauri/src/commands.rs`（`filter_run` 入参加 `time: Option<TimeSpecJson>`/`line_from`/`line_to`；组装 `line_range=Some((from,to))`仅当两侧都给、`time_core=time.to_core()`；no-op 守卫改为“无任何轴”才短路；`LineFilter::build(specs,case,time_core,line_range)`；新 2 测）；Modify `dto.rs`（剥 TimeSpecJson+to_core 的 `#[allow(dead_code)]`）。
**Why:** spec §3.3—时间/行号与文本 AND 同扫描一次完成（单一选中集），不二次 pass。
**验证:** `cargo test -p lumelog-app` 28 pass（新 `scan_filter_line_range_degrade`、`scan_filter_time_axis_keeps_range`）+ `cargo clippy --workspace --all-targets -- -D warnings` 净；前端未改。

### Task 5. `export_lines` / `export_cancel` 命令
**Files:** Modify `app/src-tauri/src/commands.rs`（新 `open_export_target` + `#[tauri::command] export_lines`（同步 prologue 校验/建文件 → `spawn_blocking` 持 doc 锁跑 `export::run` 写原始字节、`on_progress` 节流、`on_done` 始终发、`end_search`）+ `export_cancel`；+`export_cmd_tests`）；Modify `app/src-tauri/src/lib.rs`（注册两命令）；Modify `dto.rs`（剥 ExportProgress/ExportStatsJson/From 三处 `#[allow(dead_code)]`）。
**Why:** spec §3.3/§5：目标路径打开失败同步返 `io`（不空写）；worker 持 doc 锁整任务扫描（同 `can_parse`/`filter_run`，主线程不冻），读/写错吸收进 `errored`、取消保留前缀，`on_done` 必发。**Ruling：**worker 持 `doc` 锁期间若同时关文档，`doc_arc` 被任务持有→后端扫完已关文件（与增量1 同模），不阻塞交付（已记 out-of-scope，二增量可接 export 取消 worker）。
**验证:** `cargo test -p lumelog-app` 30 pass（新 `open_export_target_bad_path_is_io`/`open_export_target_creates_file`）+ `cargo clippy --workspace --all-targets -- -D warnings` 净；`use std::io::Write` 补入使 `flush` 可用。

### Task 6. 前端 IPC 桥（时序/导出）
**Files:** Modify `app/frontend/src/ipc.ts`（新 `TimeFormat`/`TimeSpec`/`ExportStats` 类型；`filterRun` args 加 `time?/lineFrom?/lineTo?` 并透传；新 `exportLines`（多 Channel、解为 req_id）/`exportCancel`）；Modify `app/frontend/src/ipc.test.ts`（+`export_wrappers_are_functions`）。
**Why:** spec §4.2；镜像 Rust 命令参数（camelCase↔snake）；错误码全复用。
**验证:** `npm --prefix app/frontend test` 47 pass（ipc 9→10）+ `run build`（tsc 通过，41 modules）；Rust 未改。

### Task 7. 查询面板时序区 + 导出
**Files:** Create `app/frontend/src/query.ts`（纯函数 `pickTimelineInput`：抽取正则非空→时间轴(from/to可单侧)，否则行号区间(两侧必填/正整数/from≤to)，错误返 `error` 不抛）+ `query.test.ts`（7 例）；Modify `app/frontend/src/main.ts`（`applyQuery` 加 `sel: TimelineSelection` 参、no-op 守卫改为“无任何轴”、传 time/lineFrom/lineTo；新 `exportQuery`；面板加时序行(`query-time`：抽取正则+格式下拉+起止+保留无ts)+“导出…”按钮(save 对话框、无轴时 `ask` 二次确认)；共享“停止”按 `queryKind` 分派 `filterCancel`/`exportCancel`）。
**Why:** spec §4.1。导出与扫描共用单任务互斥（queryBusy/queryReqId），`queryKind` 决定 Stop 取消哪个；时间轴/行号退化由 `pickTimelineInput` 集中判定（可测）。
**验证:** `npm --prefix app/frontend test` 54 pass（新 `query.test.ts` 7 + ipc 10）+ `run build`（tsc 通过，42 modules）。

### Task 8. 样式 + 全量验收
**Files:** Modify `app/frontend/src/styles.css`（`.query-time` 时序行：flex-wrap、`input[placeholder^="抽取"]` 增长、起/止窄；按 placeholder 定位避免误命中勾选框）。
**Why:** spec §7。面板复用 `#search-panel` flex 列（head/conds/timeRow/status），时序行固定不抢 conds 滚动区。
**手工验收清单（需 `tauri dev`）：**① ISO 时间窗只显区间内行；② 不填抽取→行号区间退化生效；③ 保留无时间戳行开关有/无效；④ 导出当前筛选→目标文件行内容/编码/行尾与源一致（多字节/CRLF）；⑤ 大文件导出中“停止”→文件为前缀、toast 区分取消/中断/成功。
**验证（全量四门）:** `cargo test --workspace` 全绿（app 30、core 78 含 filter16/export4、can 集成、cli_smoke 9/1 ignored、docfile/proptest）+ `cargo clippy --workspace --all-targets -- -D warnings` 净；`npm test` 54 pass + `run build`（tsc 通过）。

## 2026-10-25 — Plan 4 M3 行过滤 + 高亮命中（feat/m3-filter-highlight）

**依据：** spec `docs/specs/2026-10-25-m3-filter-highlight-design.md`；计划 `docs/plans/2026-10-25-m3-filter-highlight.md`（M3 子项目 A 增量1：只读视图 多条件 AND 行过滤 + 命中区高亮 + F3 跳转 + 实时取消）。

### 终审修复波（CodeReview 全分支 0 Critical / 5 Important / 6 Minor）
**Files:** Modify `app/frontend/src/listview.ts`、`app/frontend/src/main.ts`。
**Why（逐条）：**①F3 被 window 层无条件劫持、与编辑器 CodeMirror searchKeymap 双响应 → 改为仅当活动文档是只读列表（`lists.has`）才接管，否则放行编辑器；②`handleClose` 漏清 `queryHits/queryCursor` 且迟到的 `onDone` 会重填已闭文档（docId 单调不复用→永久泄漏）→ handleClose 补删两图 + onDone 首行 `docStates.has` 守卫；③子集模式下 `scrollToLine/revealLine` 未按 hits 重映射、CanPanel/Ctrl+G 跳转错位且 `paintSubset` 不读 `flashIndex` → 新增 `slotForLine` 二分映射 + 子集 `paintSubset` 闪烁判定；④换编码后沿用旧解码算出的 cu16 区间与旧命中行集（同“行文本一致”约束的编码维度）→ `changeEncoding` 列表分支 `setView(null)`+清 hits+提示；⑤扫描中重开面板丢弃“停止”入口并把流式计数写进已 detach 旧节点 → `openQueryPanel` 在 `queryBusy` 时复用现有 DOM不重建。另修 Minor：⑦子集取文回传 `invalidCount` 使 §5.4 ⚠ 不再冻结（loadLines 改为返回全 payload）；⑨`await filterRun` 后补 `if(!queryBusy) queryReqId=null` 防早完成写完已结束 id；⑳声明 `queryBusy` 为全局单扫描互斥的有意行为。**Ruling（park）：**⑥“整行淡色”截断退化本轮只留计数提示（区分“截断空区间”与“仅排除空区间”需额外标记，价值低）→ M3 第2增量；⑪命中行以 JS 对象数组驻留（百万级约 100MB而非 spec 估的“数 MB”），典型命中稀少可接受，建议二增量改 `Int32Array`+稀疏区间 Map。Minor⑧（`setView(null)` 无调用方）由修复④自然获得调用。
**验证:** `npm --prefix app/frontend test` 46 pass + `run build`（tsc 通过）；Rust 本轮无改动（上轮 workspace 测试+clippy 已净）。待定向复审（fix range）后 merge main。

**定向复审（CodeReview，fix range 73573f4..a924da5）：** 8 项待验证 finding（1/2/3/4/5/7/9/10）全部 ADDRESSED，Minor 8 的 `setView(null,...)` 调用方已确认落地；无新 Critical/Important。修复波自引入两条非阻塞 Minor——**Ruling（park）**：A 子集模式 ⚠ 改为“增量批次计数”与连续模式口径不一（较修复前“完全冻结”已改善，完整累计待二增量）；B 复用面板在“跨文档切换+扫描中”时归属不明（全局单扫描互斥下的 UX 代价）；均不阻塞 merge。Out-of-scope 遗留（非本轮回归）：扫描中切编码后迟到 onDone 会用旧解码 hits 重填、handleClose 不取消在途 worker（Rust 侧会跑完已关文档）——记入后续。

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
