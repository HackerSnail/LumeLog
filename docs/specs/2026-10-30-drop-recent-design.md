# 拖放打开 + 最近文件 设计（Plan 20）

偿还 test-checklist §4 记录的 M1 欠账：「拖放打开、最近 20 文件菜单：M1 未接线，归 Plan 2 GUI 打磨」。
Notepad++ 替代定位下，这两个是基本操作习惯。

## 1. 拖放打开（前端为主）

- Tauri v2 不暴露 File 路径给 webview，走 `getCurrentWebview().onDragDropEvent`
  （`enter`/`over`/`drop`/`leave` 四态；`core:default` 能力已含事件监听，窗口
  `dragDropEnabled` 默认开，无需改 capability）。
- `enter`/`over`：全屏遮罩「松开以打开文件」；`leave`/`drop`：隐藏。
- `drop`：对 `paths` **顺序** `await openPath(p)`——`openPath` 自带 busy 单飞与错误 toast，
  首个失败即停（与单开语义一致）；成功打开的文件照常进标签页。
- 遮罩 div 在 `layout()` 建，样式新类 `.drop-overlay`（fixed、半透明、虚线框）。
- 动态 `import("@tauri-apps/api/webview")`，非 Tauri 环境（纯 vite dev）安全降级为不监听。

## 2. 最近文件（Rust 为主）

- 存储 `$APP_CONFIG/LumeLog/recent.json`：`["path", …]`，**在 `doc_open` 成功后由后端记录**
  （拖放/对话框/palette 全部入口自动覆盖），去重提到最前，上限 20，tmp→rename 原子写。
- `recent_list()`：读回时过滤已不存在的路径（fs 检查 ≤20 次可忽略）；文件缺失/损坏 → 空表。
- `recent_clear()`：清空。
- 前端入口：命令面板顶部注入「最近：<文件名>」（hint 显示完整路径，最多 10 条）+
  「清除最近文件记录」；启动时读一次，每次成功打开后刷新。
- 不做：侧栏最近文件列表（palette 已覆盖，UI 再加一处需维护两份状态）、固定/置顶。

## 3. 验收

- Rust `recent.rs` 单测：记录去重置顶、上限截断、损坏 JSON/缺失文件容忍、clear、原子写不留 tmp。
- `doc_open` 成功后 recent 更新（集成点由前端行为覆盖 + 单测直测 record 函数）。
- 前端：`tsc --noEmit` + `npm test` + `npm run build` 绿；ipc typeof +1。
- 人工：`tauri dev` 拖一个 GBK 文件进窗口打开不乱码；palette 见最近列表并跳转。
