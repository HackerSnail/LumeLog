# M3 子项目 C：快捷键自定义 设计

> 依据主 spec `docs/specs/2026-10-24-lumelog-v1-design.md` §8（“快捷键：默认方案（Ctrl+F/Ctrl+Shift+F/Ctrl+G/Ctrl+O/Ctrl+S…）+ 用户覆盖表”）、§10（`config.toml`：主题、快捷键覆盖…）、§12 M3（主题/快捷键/layout）。承接已交付：命令面板（`buildCommands` 的稳定 `id` + `ACTIONS` 可抽取）、`config.toml` 最小读写（`recovery_enabled`，`commands.rs::config_path`）、jsdom DOM 测试基建。

**Goal:** 用户可在命令面板“快捷键设置…”中为 13 个应用级动作录制/清除/重置快捷键；覆盖持久化到 `config.toml` 的 `[keymap]` 表；全局快捷键按生效表分发，面板 hint 显示生效键；非法/冲突绑定被拒绝而不是静默生效。

**Architecture:** 后端 `app/src-tauri/src/keymap.rs` 纯函数（`read_keymap`/`set_binding`/`reset_keymap`，TOML 表，保留其它配置键）+ 3 命令（`keymap_get/set/reset`）。前端 `keymap.ts` 纯逻辑（默认表、`eventToAccel`/`parseAccel`/`formatAccel`、`validate`、`mergeBindings` 含冲突）+ `keymap-ui.ts` 录制模态；`main.ts` 把 `buildCommands` 的 run 抽为共享 `ACTIONS`，`registerShortcuts` 改为查 `accel→action` 表分发，启动读覆盖并合并。

**Tech Stack:** Rust（复用 `toml`，无新依赖）+ TypeScript（无新依赖；测试用已装 jsdom 仅当需要 DOM）。门：`cargo test` / `cargo clippy --workspace --all-targets -- -D warnings` / `npm test` / `run build`。

## 1. 范围与非目标
- 做：13 个可绑定动作；`config.toml` `[keymap]` 覆盖（缺失→默认，`""`→解绑）；录制模态（录制/清除/单项恢复/全部恢复、冲突拒绝、Esc 取消）；全局分发与面板 hint 生效值；校验规则防打字劫持。
- 不做：跨设备同步（v1 明示后续）、多段和弦（`Ctrl+K Ctrl+S`）、按上下文分支绑定、CodeMirror 内建键位（Ctrl+F 等）与 F3/Shift+F3 命中循环（保持固定，设置模态注明“不可绑定”）。

## 2. 动作与默认键（单一来源）
| id | 标题 | 默认 |
|---|---|---|
| `open` | 打开文件… | Ctrl+O |
| `save` | 保存 | Ctrl+S |
| `save-as` | 另存为… | Ctrl+Shift+S |
| `query` | 行过滤 / 高亮命中… | Ctrl+Shift+F |
| `jump` | 跳转到行… | Ctrl+G |
| `palette` | 命令面板 | Ctrl+Shift+P |
| `close` | 关闭当前文档 | （无） |
| `sidebar` | 显示 / 隐藏侧栏 | （无） |
| `inspector` | 显示 / 隐藏文档信息栏 | （无） |
| `can-panel` | 报文面板：显示 / 隐藏 | （无） |
| `can-import` | 导入 DBC… | （无） |
| `can-parse` | 解析当前文档为报文 | （无） |
| `can-snippet` | 识别选中为报文 | （无） |

`main.ts` 抽 `ACTIONS: Record<ActionId, () => void>`，`buildCommands()` 由它生成（title/hint 来自动作元数据表），全局键盘分发也调它。

## 3. 加速键规范（`keymap.ts`）
- 规范串：修饰键顺序 `Ctrl+Alt+Shift+Key`；`Ctrl` token **同时匹配 Ctrl/Cmd**（跨平台主修饰，显示统一 `Ctrl`）；字母大写；空格→`Space`；命名键保持（`F1..F24`/`Escape`/`ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`/`Backspace`/`Delete`/`Enter`/`Tab`/`Home`/`End`/`PageUp`/`PageDown`/`Insert`）。
- `eventToAccel(e:{key,ctrlKey,altKey,shiftKey,metaKey}) -> string | null`：仅修饰键按下→`null`；其余按上述规范化。
- `parseAccel(s) -> {ctrl,alt,shift,key} | {error}`；`formatAccel` 逆变换；`normalizeAccel`（排序/大小写归一）。
- `validate(accel) -> {ok:true} | {ok:false,reason}`：**必须含 Ctrl 或 Alt，或 key 为 `F1..F24`**（拒绝裸字母/裸 Escape/Enter/Tab/Space/Backspace/Delete，避免劫持打字）；拒绝空串（空串在覆盖表中表示“解绑”，不参与校验）。
- `mergeBindings(overrides: Record<string,string|undefined>) -> {bindings: Record<ActionId,string>, accelToAction: Record<string,ActionId>, conflicts: {action,other,accel}[], unknown: string[]}`：以默认表为基，`""`=解绑（从 accel 表移除），未知 action 记入 `unknown` 并忽略；**冲突先到先得**，后者从绑定中剔除并记 `conflicts`（UI 提示，不阻断启动）。`conflictOf(accel, bindings, selfId) -> ActionId | null` 供录制时查重（排除自身）。
- 分发：`accelToAction` 由合并结果构建；`keydown` 时 `eventToAccel` → 查表 → 命中即 `preventDefault` 并执行。

## 4. 后端（`app/src-tauri/src/keymap.rs` + 命令）
- 纯函数：
  - `read_keymap(text:&str) -> BTreeMap<String,String>`：`toml::Table` 的 `keymap` 表；非字符串值跳过；表缺失→空。
  - `set_binding(text:&str, action:&str, accel:Option<&str>) -> String`：`None`→删除该键（回默认）；`Some("")`→写空串（解绑）；保留其它配置键与其它动作条目；坏 TOML→重建为仅该表。
  - `reset_keymap(text:&str) -> String`：删除 `keymap` 表，保留其它键。
- 命令（复用 `commands.rs::config_path` 读写）：
  - `keymap_get() -> Result<BTreeMap<String,String>,IpcError>`
  - `keymap_set(action:String, accel:Option<String>) -> Result<(),IpcError>`
  - `keymap_reset() -> Result<(),IpcError>`
  - `lib.rs` 注册；`mod keymap;`。

## 5. 前端 UI（`keymap-ui.ts`）
- `openKeymapModal(opts: { actions: {id,title}[], bindings: Record<string,string>, onSet:(action,accel:string|null)=>Promise<void>, onResetAll:()=>Promise<void>, onClose:(changed:boolean)=>void })`。
- 行：标题 + 当前生效键（或“未绑定”）+ “录制” + “清除” + “恢复默认”；录制态显示“按下新快捷键…（Esc 取消）”。
- 录制：`keydown` 捕获（`preventDefault/stopPropagation`），Esc→取消；`eventToAccel`→`validate`→`conflictOf(accel, bindings, selfId)`：重复→行内红字“已绑定给《保存》”并保持录制；成功→`onSet(id, accel)` 后更新行。
- 清除→`onSet(id, "")`；恢复默认→`onSet(id, null)`（后端删键）；“全部恢复默认”→`onResetAll()`。
- 模态打开时**独占键盘**：全局分发在 `keymap-ui` 打开期间短路（由一个 `keymapModalOpen` 标志）；Esc/遮罩关闭，`onClose(changed)` 触发一次应用刷新。
- 命令面板新增 `{ id:"keymap", title:"快捷键设置…" }`，`hint` 用生效键；面板行提示“（F3 / Ctrl+F 等上下文键不可自定义）”。

## 6. 错误 / 边界
- 文件里未知 action / 非法 accel：忽略并在模态顶部提示一次（`conflicts`/`unknown`）；原值不删（等用户显式改）。
- 冲突：启动合并先到先得（后者失效并提示）；录制直接拒绝。
- 系统保留键（Ctrl+W 等）：允许但模态注明“可能被系统占用”。
- `accel=""`：解绑，动作仅能从命令面板触发。
- 写配置失败：toast 报错、内存不改（保持旧表）。
- `Ctrl` 与 `Cmd` 等价匹配（mac 上 Cmd 生效）；显示统一 `Ctrl`。

## 7. 验收
**可测（Rust）：** `read_keymap`（正常/缺表/坏值跳过）、`set_binding`（保留 `recovery_enabled`/`theme` 等其它键；`None` 删键；`Some("")` 解绑；坏文本重建）、`reset_keymap`（只删表、保留其它键）。
**前端（headless；录制的 DOM 片段可选 jsdom）：**
- `eventToAccel`：Ctrl/Cmd 等价、Alt/Shift 顺序、空格与命名键、仅修饰→null。
- `validate`：裸字母/Escape 拒；`F3`、`Ctrl+Shift+F`、`Alt+1` 通过；空串单独处理（解绑）。
- `mergeBindings`：默认/覆盖/`""` 解绑/冲突先到先得/未知 id；`conflictOf` 查重（排除自身）。
- `ipc` 三包装 typeof。
**GUI 手工：** 改 Ctrl+S→Ctrl+Shift+S：新键生效、旧键失效；与保存冲突的录制被拒；清除后仅面板可触发；重启保留；全部重置回默认；模态打开时其他快捷键不触发。

## 8. 全局约束（沿用）
简体中文 UI；命令驼峰↔下划线自动；逐任务 TDD；四门绿再提交；commit 前追加 CHANGELOG；无空格 commit 消息；commit 后 push；分支 `--no-ff` 合 main。

## 9. 分期（计划切分预告）
① 后端 `keymap.rs` 纯函数 + 测 → ② 3 命令 + 注册 → ③ 前端 `keymap.ts`（默认表/eventToAccel/validate/merge）+ 测 → ④ `ipc.ts` 三包装 + 测 → ⑤ `main.ts` 抽 `ACTIONS` + 表驱动分发 + 启动加载/面板 hint/keymap 命令 → ⑥ `keymap-ui.ts` 录制模态 + 样式 + 持久化接线 → ⑦ 收尾四门 + 终审 + 合 main。

## 10. 关键取舍（已定案）
- 复用命令面板 `id` + `ACTIONS`（单一动作源，避免两套注册）。
- `Ctrl` 统一代表主修饰（匹配 Ctrl/Cmd，显示不分平台）——v1 简化。
- 仅 `[keymap]` 表 + 录制模态；`recovery_enabled` 现有布局不动。
- 校验要求主修饰或 F 键（防劫持打字）；上下文/CodeMirror 键位不纳入。
- 冲突“先到先得 + 提示”而非自动交换（可预测、实现简单）。
