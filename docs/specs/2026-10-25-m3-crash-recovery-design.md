# M3 子项目 B1：未保存编辑的崩溃恢复 设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` §4.4（崩溃恢复：脏文档停输 2s 写影子文件含编码/行尾元数据；启动扫描 recovery 逐个询问恢复，恢复后仍脏；编辑模式全量、只读不写）与 §10（`$APP_CONFIG/LumeLog/recovery/`；正常保存/关闭后删除；`recovery_enabled` 可关）、§12 M3 验收（kill -9 后可恢复脏文档）、§15 A7。承接 M1–M3（`doc_save` 原子写、`EditorHandle.getText`、`DocState{path,encoding,eol,mode}`、`tabs.refreshDirty`）。

**Goal:** 编辑模式的脏文档在停止输入 2s 后把全量文本 + 编码/行尾元数据写入影子文件；异常退出后启动时以**单列表横幅**发现并逐项"恢复/丢弃"；恢复后文档仍脏、保存才落盘；正常保存/丢弃后删除影子。只读模式不产影子。

**Architecture:** 后端 `lumelog-app` 新增 `recovery` 模块（路径 `app_config_dir()/LumeLog/recovery`，每文档一影子 `<doc_key>.lumelog`：首行 JSON 元数据 + `\n` + 原始 UTF-8 文本；`doc_key = FNV-1a64(path)` hex；tmp→rename 覆盖），四命令 `recovery_save/list/load/discard`。前端 `main.ts` 每 docId 一个 2s debounce（`onDirty` 且编辑器脏时重置；到点取 `handle.getText()` + `ds.encoding/eol` 调 `recovery_save`），启动后 `recovery_list()` 非空则显示固定横幅列表；恢复=打开原文件→`replaceText(shadow)`→套影子 encoding/eol→标脏→丢弃旧影子。保存成功与关闭丢弃时 `recovery_discard`。

**Tech Stack:** Rust（Tauri v2 `app.path().app_config_dir()`；新增 `toml`（仅读/写 `config.toml` 的 `recovery_enabled`）；FNV-1a 手写）+ TypeScript 原生 DOM。门：`cargo test`、`cargo clippy --workspace --all-targets -- -D warnings`、`npm test`、`run build`。

## 1. 范围与非目标
- 做：编辑模式脏文档影子（2s 节流、全量文本、编码/行尾元数据）；启动单列表横幅 + 恢复/丢弃（含“全部丢弃”）；恢复回填并保持脏；保存/丢弃生命周期清理；后端四命令 + 可测的存储层；**最小配置开关** `config.toml` 的 `recovery_enabled` 单键 + 侧栏勾选框。
- 不做：外部修改监听（B2）；完整 `config.toml` 配置系统（只做 `recovery_enabled` 单键，其余键留待后续）；只读文档恢复（spec：只读无可恢复内容）；恢复多版本/历史（只保留最新一次影子）。

## 2. 存储格式（后端）
- 目录：`app_config_dir()/LumeLog/recovery/`，命令首次使用时 `create_dir_all`。
- 文件名：`format!("{:016x}.lumelog", fnv1a64(path))`。`fnv1a64` 对 `path` 的 UTF-8 字节做标准 FNV-1a 64 位。
- 文件内容：**首行** `serde_json` 序列化的 `RecoveryMeta { path, encoding, eol, saved_at_ms }` + `b'\n'` + **原文 UTF-8 字节**（不做 JSON 转义；大文本友好）。
- 写入：同目录 `<doc_key>.tmp` 写入→`rename` 覆盖（影子非权威数据，不强制 fsync）。读取：按首个 `\n` 切分（首行缺失/JSON 坏 → 该影子视为损坏，`recovery_list` 跳过并在日志/返回中计数）。
- 解析/文本编码：影子文本恒为 UTF-8（CodeMirror 内部 LF 归一）；`encoding`/`eol` 仅作恢复后保存用的元数据。

## 3. 后端命令（`lumelog-app`，`lib.rs` 注册）
所有命令以 **`path`（原文件路径）** 为键，后端内部用 `doc_key(path)` 推导影子文件名；前端不接触 FNV。
- `recovery_save(path:String, encoding:String, eol:String, text:String) -> Result<(),IpcError>`（写/覆盖）。
- `recovery_list() -> Result<Vec<RecoveryEntry>,IpcError>`：`RecoveryEntry{path, encoding, eol, saved_at_ms:u64, size_bytes:u64}`（camelCase `savedAtMs/sizeBytes`）；目录不存在→空 vec。
- `recovery_load(path:String) -> Result<RecoveryContent,IpcError>`：`RecoveryContent{path, encoding, eol, text:String}`；坏文件/缺失→`io`。
- `recovery_discard(path:String) -> Result<(),IpcError>`：删除；不存在视为成功（幂等）。
- 纯逻辑（可测）：`fn fnv1a64(bytes:&[u8])->u64`、`fn doc_key(path:&str)->String`（16 位小写 hex）、`fn shadow_path(dir:&Path, path:&str)->PathBuf`、`fn write_shadow(dir:&Path, meta:&RecoveryMeta, text:&str)->io::Result<()>`、`fn read_shadow(dir:&Path, path:&str)->io::Result<(RecoveryMeta,String)>`、`fn list_shadows(dir:&Path)->Vec<RecoveryEntry>`（损坏跳过）、`fn discard_shadow(dir:&Path, path:&str)`（幂等）。命令薄壳用 `app.path().app_config_dir()` 拼 `LumeLog/recovery`。

### 3.1 最小配置开关（`config.toml`）
- 文件 `$APP_CONFIG/LumeLog/config.toml`，顶层 `recovery_enabled: bool`；缺失/解析失败→视为 `true`（默认开启）。
- 纯逻辑：`fn read_recovery_enabled(text:&str)->bool`（`toml` 解析，缺失/坏→true）、`fn set_recovery_enabled(text:&str, enabled:bool)->String`（保留其它键：`toml::Table` 读→改键→`toml::to_string_pretty`；原文本坏则重写为仅该键）。
- 命令：`recovery_enabled() -> Result<bool,IpcError>`、`recovery_set_enabled(enabled:bool) -> Result<(),IpcError>`（读→改→写，`create_dir_all`）。
- 禁用时行为：`recovery_save` **no-op Ok**（防御）；`recovery_list` 返回空（不显示横幅）；**既有影子不删**（重新开启后可恢复）；前端也不排定时器。

## 4. 前端（`main.ts`）
- **节流**：`const recoveryDebounce = new Debouncer(2000, fireRecovery)`（`recovery.ts`，按 docId 合并）。在 `mountDoc` 的 `onDirty` 回调里：先 `tabs.refreshDirty(docId)`（保留），若 `editors` 有该 doc 且 handle `isDirty()` 且开关开启 → `recoveryDebounce.schedule(docId)`；到点 `recovery_save(ds.path, ds.encoding, ds.eol, handle.getText()).catch(...)`（失败仅 `status`/toast 提示一次，不阻塞）。
- **清理**：保存成功（`afterSave` 内，Save-As 前后 path 都清）→ `recovery_discard(path)` + 清定时器；`handleClose` 用户选择丢弃/保存后关闭 → 同样清理；`changeEncoding` 丢弃脏改动重载后（内容回到磁盘态）也清定时器并 `recovery_discard`。
- **启动**：`layout()` 完成、`appPing` 之后 `recovery_list()`；非空 → 右上/底部固定**横幅**：`发现 N 个未保存恢复项` + 展开列表（`path` 尾部 + `savedAtMs` 本地时间 + `sizeBytes` 人类可读）每行"恢复"/"丢弃"，另有"全部丢弃"。横幅用原生 DOM（复用现有 `.banner`/`toast` 样式或新增 `.recovery-banner`）；不阻塞交互。
- **恢复**：点“恢复”→ `docOpen(entry.path)`（走既有打开流程；`unknown`/不存在→该项提示“原文件已缺失，无法恢复”，保留条目让用户显式丢弃）；成功→用 `recovery_load(entry.path)` 取文本→`editors.get(docId)!.replaceText(content.text)`→`docStates.encoding/eol = meta`→保证脏标记（`replaceText` 已触发 `onDirty`）→`recovery_discard(entry.path)`→从横幅移除该项。
- **丢弃**：`recovery_discard(entry.path)` 后移除；全部丢弃清空横幅。
- **开关 UI**：侧栏（`aside`）底部新增 `.side-option` 复选框“崩溃恢复”（`#recovery-enabled`）。启动时 `recoveryEnabled()` 回填（失败→勾上，默认开）；变更时 `recoverySetEnabled(v)`（失败→回滚勾选 + toast），并在关闭时 `recoveryDebounce.cancelAll()`；关闭状态不排新影、不显示横幅。
- 只读文档：`editors` 无该项 → 不排定时器、不写影子。

## 5. 错误 / 边界
- 影子写失败：`catch` → 状态栏提示（非阻断），下次停输重试；不抛模态。
- `app_config_dir` 不可用：命令返回 `io`；前端 `recovery_list` 失败仅记日志、不显示横幅（不阻塞启动）。
- 损坏影子：`recovery_list` 跳过；`recovery_load` 报 `io`，该项提示损坏。
- 原文件缺失/已改名：恢复项提示缺失、**不自动删**（用户显式丢弃）。
- 大文本：2s 合并写、全量（编辑阈值内）；崩溃时最多丢最后 2s 输入（spec 既定节流）。
- 同一路径重开：同一 `doc_key` 覆盖（最新胜出）。

## 6. 验收
**可测（Rust）：**
- `fnv1a64` 已知向量（空串 = `0xcbf29ce484222325`，"a" = `0xaf63dc4c8601ec8c`）；同 path 稳定。
- `write_shadow`→`read_shadow` 往返：meta 四字段 + 文本逐字符（含换行/中文/emoji）；首行格式正确。
- `list_shadows`：多条目按 `saved_at_ms` 降序、坏文件跳过、目录缺失→空。
- `discard` 删除且幂等。
**前端：**
- 抽 `recovery.ts` 纯 helper（`humanSize(bytes)`、`formatSavedAt(ms)`、debounce 调度器）→ vitest。至少测 debounce 调度器（fake timers：连发只写一次、清定时器不写）与 `humanSize`/时间格式边界。
- `ipc.ts` 四包装 typeof + 参数透传 camelCase（均以 `path` 为键）+ `recoveryEnabled`/`recoverySetEnabled`。
- 配置：`read_recovery_enabled` 缺失/坏文本→true；`set_recovery_enabled` 保留不相关键（如 `theme="dark"`）且幂等；真写盘后再读回一致。
**GUI 手工（`tauri dev`）：**编辑脏文档→停 2s→`recovery/` 出现影子；`kill -9` 重启→横幅列出→恢复→文本回填且仍脏→保存后影子消失；丢弃→影子删；只读文档不产影子；坏影子被跳过。

## 7. 全局约束（沿用）
简体中文 UI；DTO camelCase + Option `skip_serializing_if`；命令驼峰↔下划线自动；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；无空格 commit 消息；commit 后 push；分支 `--no-ff` 合 main。

## 8. 分期（计划切分预告）
① 后端 `recovery` 存储层（fnv1a64/write/read/list/discard + 单测）+ 最小配置读写（`read/set_recovery_enabled` + 单测，加 `toml`）→ ② 四 recovery 命令 + 两个配置命令 + DTO + 注册 + `app_config_dir` 解析 → ③ 前端 `recovery.ts` 调度器 + 测 → ④ `ipc.ts` 桥 + 测 → ⑤ `main.ts` 接节流/保存/关闭清理 + 启用门控 → ⑥ 侧栏勾选框 + 启动横幅 + 恢复/丢弃流程（含样式）→ ⑦ 收尾四门 + 终审 + 合 main。

## 9. 关键取舍（已定案）
- 影子格式：首行 JSON meta + 余下原文（避免大文本 JSON 转义）；`doc_key` 用 FNV-1a64 hex（FNV 自身零依赖），不用 sha256。
- 启动 UX：单列表横幅（你选），非逐个 `ask`；不阻塞启动。
- 恢复后即丢弃旧影子（编辑器下次停输重写），避免"陈旧双份"。
- **最小配置开关**（你选）：只实现 `config.toml` 的 `recovery_enabled` 单键 + 侧栏勾选框；其余配置系统（主题/快捷键/阈值）不在本增量。新增 `toml` 依赖仅用于该键的读/写（保留其它键）。禁用时既有影子保留、可重开后恢复。
- 影子文本恒 UTF-8/LF；`encoding`/`eol` 仅元数据，恢复后按原值保存。
