# 文件关联 / 单实例（Plan 43）

双击 .log 直接用 LumeLog 打开；重复启动唤起已有窗口并把新 argv 里的
文件交给它。Windows 为主目标，关联配置对 macOS/Linux bundle 一并生效。

## 1. 单实例（Rust）

- `tauri-plugin-single-instance`（须第一个注册）：第二实例启动时回调
  拿到 argv，`launch::from_argv` 过滤（跳过程序名与 `-` 开头参数），
  `emit("second-instance-open", paths)` 给前端并 `set_focus()` 主窗口；
- `launch.rs`：进程启动时 `collect()` 存初始 argv（OnceLock），
  命令 `launch_args()` 返回——前端启动时打开双击传入的文件。

## 2. 文件关联（bundle 配置）

- `tauri.conf.json` 加 `bundle.fileAssociations`：`.log`（Editor 角色，
  mimeType text/plain）——NSIS/DMG/deb 按各自机制注册；.blf/.asc 走
  CAN 面板的独立入口，不进关联（打开语义不同）。

## 3. 前端

- 启动：`launchArgs()` → 逐个 `openPath`（非 quiet，双击打开的报错
  用户要看见）；监听 `second-instance-open` 事件逐个打开；
- 会话恢复（Plan 40）已在启动序列，二者叠加：argv 文件开在会话之后
  （双击意图优先）。

## 4. 验收

- Rust 单测：from_argv 过滤规则；clippy + workspace 测试净；
- `tauri build` 通过（验证 bundle 配置合法 + NSIS 关联注册不报错）；
- 人工：双击 .log 打开、二次启动唤起窗口（Windows 实机）。
