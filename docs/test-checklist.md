# M1 验收清单（LumeLog）

三平台 = Windows(main 目标) / macOS / Linux。标 ✅ 的须填实测数字；
自动化可跑项（P3/P4/P5/编译矩阵）由 CI + headless 工具产出；P1/P2（冷启动/首屏）
须真实 GUI 计时，headless 不可测，故 §1 诚实标注为**未验证**（`app_ping` 已作 P1 计时终点）。
性能门槛原值见 spec §2（docs/specs/2026-10-24-lumelog-v1-design.md）。

## 1. 性能实测记录表

| 编号 | 指标 | 门槛 | Windows 实测 | macOS 实测 | Linux 实测 |
|---|---|---|---|---|---|
| P1 | 应用冷启动到可交互 | ≤800ms | **未验证**（真实 GUI 秒表，headless 不可测；`app_ping` 已埋计时终点） | 未验证 | 未验证 |
| P2 | 1GB 日志首屏可见 | ≤500ms | **未验证**（打开文件到首行渲染，需 tauri dev） | 未验证 | 未验证 |
| P3 | 1GB 索引耗时 / 内存增量 | ≤3s / ≤400MB | **654ms / 峰值RSS 63MB**（全进程 open=建索引+行数；index-floor 64MB） | 补测 | 补测 |
| P4 | 取块+解码 | ≤16ms P95 | **p50=0.153ms · p95=0.230ms · p99=0.259ms**（cargo bench chunk_at_gb，1000 次随机 600 行窗） | 补测 | 补测 |
| P5 | 1GB 字面量搜索 | ≤10s | **1513ms**（全进程含 1.69 万命中逐行打印；criterion 纯扫描 ~580ms） | 补测 | 补测 |
| P6 | Windows 安装包体积 | ≤15MB | 见 §3 | n/a（产物在 CI） | n/a（产物在 CI） |

### headless 复跑命令（Windows PowerShell，仓库根）

```powershell
# 生成 1GB 定种日志（~1.08GB 字节，tests_common::gen_log(1024) 同源；写临时目录，不入仓）
cargo run -p lumelog-core --release --bin lumelog-gen -- "$env:TEMP\bench_1gb.log" 1024

# P3：打开=建全索引。计时 = 进程总耗时；内存 = 运行期间轮询 PeakWorkingSet64
Measure-Command { .\target\release\lumelog.exe open --file "$env:TEMP\bench_1gb.log" --line-count }
# 另开终端轮询： (Get-Process lumelog -EA SilentlyContinue).PeakWorkingSet64 / 1MB

# P5：字面量搜索全文件（rare pattern ~1 命中/1000 行）
Measure-Command { .\target\release\lumelog.exe search --file "$env:TEMP\bench_1gb.log" --pattern "val=999" | Out-Null }

# P4：cargo bench -p lumelog-core --bench chunk_at（p95 打印行）
```

### 实测记录（Windows，2026-10-24，i 盘 SSD，定种语料 gen_log(1024)=926 MiB / 16,777,217 行）

口径说明：P3/P5 数字为 **CLI 全进程 end-to-end**（含进程启动 + 建全行索引 + 命中逐行写 stdout），
比 criterion 隔离口径偏保守仍达标；RSS 由 80ms 轮询 tasklist 采样（短进程为下限值），
但 index-floor 已由 `行数×4B=64MB` 锚定，内存增量对 400MB 门槛有约 6× 余量。语料须为已落盘
静止文件；刚生成即测会因读操作与脏页回写争用而虚高（首轮 P3/P5 曾测得 5.9s/26.4s 即此因，已复现排除）。

| 项 | 实测 | 门槛 | 判定 |
|---|---|---|---|
| P3 open cold / warm | 528ms / 654ms | ≤3s | ✅ |
| P3 峰值 RSS | 63MB（warm；cold 采样未命中记 0） | ≤400MB | ✅ |
| P5 search 全进程 | 1513ms，matches=16948 | ≤10s | ✅ |
| P4 chunk_at p95 | 0.230ms | ≤16ms | ✅ |

## 2. 功能冒烟（人工，`npm run tauri dev`，Windows 必做；其余平台抽测）

- [ ] GBK 台架日志打开**不乱码**；状态栏显示 `GBK`；切 UTF-8 再切回，内容正确重解码
- [ ] GB18030 样例（含四字节生僻字）打开正常；`BIG5` / `Shift-JIS` 样例正常
- [ ] `EUC-KR`（韩文）、`EUC-JP`（日文）样例打开正常且自动识别（或手动切换后重解码正确）；俄文样例检出 `KOI8-U`、可手动切 `KOI8-R`
- [ ] UTF-16 LE/BE 样例编辑模式打开正常（>200MB UTF-16 搜索 v1 明确拒绝并提示）
- [ ] CRLF 日志编辑保存后磁盘仍是 CRLF；LF 文件保存后仍是 LF（EOL 保持）
- [ ] 保存冲突：外部改文件后再保存 → 弹「保存冲突」**单确认**（若同时含无法映射字符，detail 会一并披露不可逆 `?` 后果）→ 强制覆盖成功
- [ ] 另存（Ctrl+Shift+S）：选新路径保存，标签页重绑到新文件
- [ ] 不可映射：GBK 文档插入 emoji 保存 → 提示占位符 `?` 数量
- [ ] 打开 1GB 日志 → **只读虚拟列表**（状态栏 `只读 · 共 N 行`），滚动流畅、行号正确
- [ ] Ctrl+G 输入行号 → 编辑/只读两模式都能定位；>200MB 模式滚动到位不发空白
- [ ] Ctrl+Shift+F 搜索：中文 IME 合成中回车**不**误提交；命中点击跳转；>1000 截断提示
- [ ] 编码下拉切换后只读列表重新解码不乱码（走 invalidate 重取）
- [ ] dirty 圆点：改动出现、保存消失、关闭未保存有确认
- [ ] 拖放打开、最近 20 文件菜单 — **M1 未接线**（打开走 `open` 对话框），归 Plan 2 GUI 打磨（见 §4）
- [ ] 搜索取消按钮 = v1 不提供（同步 search_run，见 CHANGELOG Task 8/9 Ruling）——人工确认面板无死控件

## 3. 发布物

- [ ] `npm run tauri build`（Windows）：NSIS `.exe` + MSI 产出，路径 `target/release/bundle/`
- [ ] P6：`-setup.exe` 体积 ≤15MB，数字填入 §1
- [ ] CI（ci.yml，三平台矩阵）：workspace test + clippy -D warnings + 前端 build + vitest 全绿
- [ ] release.yml（tag 触发）：三平台 bundle 产出；上传 gitee releases 需仓库 token secret（`GITEE_TOKEN`），首次发布须人工授权
- [ ] 上传动作：**由维护者手动触发/批准**（安全敏感，不在自动流内）

## 4. 已知 deferred（归后续计划）

- 拖放打开、最近 20 文件菜单：M1 未接线，归 Plan 2 GUI 打磨（`open` 对话框已覆盖“打开”主流程）
- Save-As 后再切编码：壳侧 doc 仍绑原始文件（核心 Save-As 不重绑句柄），编码切换会重读原始内容；完整重绑归 Plan 2
- 流式索引（threaded doc_open + 8MB 进度 tick）：GUI 侧路径已就绪（占位行/refresh/Channel），shell 线程化后接通
- WKWebView/Safari IME 需叠 `keyCode!==229`（当前目标 WebView2 可靠）
- CAN/DBC 解析、行过滤、崩溃恢复、外部监听 → Plan 2/3
