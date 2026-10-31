# 宏播放变体：重复 N 次 / 运行到文件尾（Plan 31）

Plan 30 的续作，补齐 N++ 宏的两个播放入口。全部前端改动，Rust 零改动。

## 1. 重复播放 N 次

- 「宏：重复播放 N 次…」——列表面板输入次数回车（`onFreeText` 既有路），
  `parseRepeatCount` 纯函数解析：纯数字、≥1 才收，钳到 1 万
  （`MACRO_PLAY_MAX`），其余 toast 提示；
- 播放直接走既有 `playMacro(times)`。

## 2. 运行到文件尾

- handle 新增 `playMacroToEnd(): number`：连续 `playMacro(1)`，
  每次后比较文档长度与光标位置，两者都不再变化即视为无进展，停止；
  返回实际播放次数；硬上限同为 1 万次防失控
  （插入型宏文档持续增长，只有上限能兜底）；
- 「宏：运行到文件尾（无进展即停）」命令面板入口，播放完 toast 次数；
  无已录制宏 → toast。

## 3. 限制

- "无进展"以文档长度 + 光标为准：排序/大小写这类不改长度且光标不动的
  宏播一次就停，与 N++ 以光标为准的行为相近；
- 播放用会话内最近一次录制的宏，已保存宏仍走「运行已保存宏」。

## 4. 验收

- `editortools.test.ts`：`parseRepeatCount` 合法/非法/钳制；
- `editor.test.ts`：下移宏运行到文件尾在最后一行停住，返回真实次数；
- `tsc --noEmit` + `npm test` + `npm run build` 绿；Rust 零改动；
- 人工：实机播放手感（`tauri dev`）。
