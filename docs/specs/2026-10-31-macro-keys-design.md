# 宏绑定快捷键（Plan 41）

N++ 宏可绑快捷键。LumeLog 的 keymap 体系（M3 C）只认 13 个固定动作 id，
宏名是动态的，且宏本体在 localStorage——绑定也放
`localStorage["lumelog.keymap.macros.v1"]`（`Record<宏名, 规范化 accel>`），
config.toml 的 `[keymap]` 不动（Rust 侧会把未知 id 当 unknown 丢弃）。

## 1. 纯函数 `macrokeys.ts`

- parse：逐条 `normalizeAccel` + `validate`（复用 keymap.ts 的规则：
  需 Ctrl/Alt 或 F1–F24，F3/Ctrl+F 保留），坏绑定丢弃；同名键保留
  首条（accel 冲突时先到先得）；空宏名丢弃；
- `conflictMacro(accel, bindings, self)`：其它宏占用检查；
- 序列化往返。

## 2. 录制 UI（keymap-ui）

- `KeymapModalOpts` 加 `macros?: { name, accel }[]` 与
  `onMacroSet(name, accel | null)`；动作行之后渲染「宏」分区
  （宏名 + 当前绑定 + 录制/清除，无默认键）；
- 录制冲突检查扩展：动作行查宏占用、宏行查动作占用与其它宏，
  错误提示带《宏名》/《动作名》；`recording` 状态放宽为 string
  （宏行用 `macro:` 前缀区分）。

## 3. 分发与联动（main.ts）

- `registerShortcuts`：动作查完查 `macroForAccel` → 命中宏即
  `playMacro(1, ops)`；
- `buildCommands`：每个已保存宏进命令面板（标题 `宏：<名>`，
  hint 显示绑定键）；
- 「宏：删除已保存宏」删除宏时连带删绑定。

## 4. 验收

- `macrokeys.test.ts`：parse（坏 accel/重名/空名）、conflictMacro、往返；
- tsc/npm test/build 绿；
- 人工：录制绑定→按键触发宏（`tauri dev`）。
