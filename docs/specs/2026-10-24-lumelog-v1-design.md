# LumeLog v1 规格说明书（Spec）

> 状态：Draft v0.6（A7/A10 按用户决定反转，已全文一致性更新） ｜ 日期：2026-10-24 ｜ 依据：`docs/claude.md` 项目定位
>
> 本文档是 LumeLog 第一个可实施版本的完整设计规格。所有【假设】标记的决策如无异议即视为定案；【待定】仅保留不阻塞 v1 的条目。

## 1. 概述与范围

LumeLog 是面向电控、HIL、嵌入式测试工程师的跨平台轻量文本日志编辑器，基于 Tauri v2 + Rust。

**v1 范围（对应里程碑 M1–M3）**：

1. 文本编辑器核心：打开/保存、多编码、正则搜索替换（M1）
2. 超大文件流式查看与索引（M1）
3. CAN/DBC 解析：DBC 导入、candump/Vector ASC 日志解码、报文表格、过滤、导出（M2）
4. 日志行过滤/高亮标记/时序筛选、主题与快捷键配置（M3）。高亮标记 = 用户正则命中处加背景色并可跳转（同 Notepad++ Find Style）；时序筛选 = 按时间戳区间圈定日志行范围（依赖 §7 解析出的时间列，无时间列时按行号退化为区间选择）。
5. 未保存编辑内容的崩溃恢复与外部修改持续监听（M3，设计见 §4.4/§10）。

**v1 非目标（明确不做）**：

- 泛用 IDE 能力（构建、调试、Git 面板、LSP）
- 多窗口、diff 对比、监控 tail 模式（Roadmap）
- BLF 二进制日志、MF4、dat 等专有格式（Roadmap）
- 外部插件加载、内置脚本解释器（见 §12，架构只预留接缝）
- DBC 复杂度量属性（BA_DEF_ 自定义属性的完整语义）、网络管理/诊断（UDS）解析

## 2. 目标与验收指标

| 编号 | 指标 | 目标值（开发机：SSD + 16GB RAM，Windows） | 验证方式 |
|------|------|------|------|
| P1 | 冷启动到首屏可交互 | ≤ 800ms | bench 脚本计时 |
| P2 | 1GB 日志首次打开到显示首屏 | ≤ 500ms | criterion bench |
| P3 | 1GB 日志全量行索引完成 | ≤ 3s，峰值内存增量 ≤ 400MB | bench + 内存采样 |
| P4 | 大文件滚动/翻页延迟 | 预取命中时零等待；未命中时后端取块+解码 ≤ 16ms（P95，不含前端渲染抖动） | bench（读取+解码链路，IPC 另计） |
| P5 | 1GB 文件（只读模式）字面量搜索 | ≤ 10s 出全部结果，支持中途取消 | criterion bench |
| P5b | 200MB 文件（编辑模式）正则搜索替换 | 结果即时逐个返回不卡 UI，替换后文档状态正确 | 集成测试 + 手动 |
| P6 | 打包体积（Windows x64 安装包） | ≤ 15MB | CI 产物检查 |

【假设】性能基准机为 i5 级 / SSD / 16GB，三平台各测各的发布门槛在 CI 中记录，不以最弱平台卡死。

## 3. 总体架构

```
┌────────────────────────── Tauri Window ──────────────────────────┐
│  Frontend (TypeScript + Vite)                                     │
│  ├─ TabBar / StatusBar / CommandPalette / FilterDialog            │
│  ├─ EditorView（CodeMirror 6，≤编辑阈值）                          │
│  ├─ ListView（自研虚拟列表，>编辑阈值，只读）                       │
│  └─ CanPanel（报文表格，TanStack Table + 虚拟滚动）                 │
├──────────────── IPC（invoke + channel 流式）──────────────────────┤
│  Backend (Rust)                                                   │
│  ├─ crates/lumelog-core   文档引擎/行索引/编码/搜索（纯 Rust，可单测）│
│  ├─ crates/lumelog-can    DBC 解析/CAN 日志解析/信号解码（纯 Rust）   │
│  └─ src-tauri/            Tauri 命令层、文件对话框、配置持久化        │
└───────────────────────────────────────────────────────────────────┘
```

设计原则：

- **重逻辑全部在 Rust**：编码、索引、搜索、CAN 解码均为独立 crate，不依赖 Tauri，可 CLI 复用、可基准测试。
- **前端薄**：只做渲染与交互；CodeMirror/Monaco 的选择见 §8。
- **数据按块流动**：后端永不把整个大文件交给前端；前端永不重复解码。

## 4. 文档引擎（lumelog-core::document）

### 4.1 数据模型

- 磁盘上的原始字节是唯一真相（source of truth）。
- `Document` 持有：文件路径、原始字节长度、`mtime + size`（外部修改检测）、编码标签、行索引。
- **行索引** `LineIndex`：`Vec<u32>`（v1 单文件上限 4GB，u32 偏移够用；超限文件拒绝打开并提示【决定】），每行起始字节偏移（含首行偏移 0）。由后台线程用 `memchr` 扫描 `0x0A` 分块构建（8MB/块），边建边可用——UI 先显示已索引部分，进度条在状态栏。空文件计为一行（内容为空）；CRLF 中 `\r` 不独立成行（仅 `0x0A` 分割，与 CodeMirror 行语义一致）。
  - GBK/GB18030 的第二字节范围含 `0x0A` 的理论风险：GBK 第二字节为 0x40–0xFE（不含 0x0A），GB18030 四字节尾字节可为 0x30–0x39。对策：行边界仅用于展示切分，误切只会把一行拆成两行显示；若某行在当前编码下解码失败且失败点是截断的多字节序列，自动与下一行合并重试（与 §5.4 错误策略配合，搜索不依赖行边界所以不受影响）。
- 索引缓存：路径 + mtime + size 作为 key，缓存文件写入系统临时目录（不落用户目录），命中则秒开。

### 4.2 读取与解码

- `chunk_at(line_start, line_count) -> Chunk`：按行取一段原始字节 → 按当前编码解码为 `String` → 同时返回 `char_map`：块内每个**解码后字符**的行内 cu16 偏移与所属行号（抽样压缩存储：连续 ASCII 段等步长合并，非 ASCII 段逐字符存；搜索命中换算靠它，见 §6.3）。
- 内存估算依据：日志主体是 ASCII，char_map 实际占用接近每行常数级；64MB LRU 上限含 char_map。
- LRU 缓存已解码块（上限 64MB），滚动往返不重复解码。

### 4.3 编辑策略（两档）

| 档位 | 条件 | 行为 |
|------|------|------|
| 编辑模式 | 文件 ≤ 200MB【假设】 | 全量读入 → 解码 → CodeMirror 文档模型；保存走 §4.4 |
| 只读查看模式 | 文件 > 200MB | 虚拟列表 + 按块读取；提供"导出片段/另存筛选结果"替代编辑 |

只读模式允许"局部转编辑"："另存选定行段为新文件后打开编辑"（行段上限仍受编辑阈值约束），作为 M3 导出片段（§7.5/§9 `export_lines`）的入口复用，不单独实现第二套取行逻辑。

### 4.4 保存

- 原子写：同目录写 `*.lumelogtmp` → `fsync` → rename 覆盖；Windows 上目标文件被其它进程锁定导致 rename 失败时，降级为"提示占用 + 另存到其它路径"，不强杀重试。
- 行尾风格：打开时检测主导风格（CRLF/LF）存入文档元数据，保存时按风格写回；混合行尾在状态栏提示并支持一键归一（全部转 CRLF/LF）。编辑模式下 CodeMirror 内部按 LF 归一，行尾仅影响保存阶段。
- 冲突处理：除保存前检查外，v1 **做持续外部修改监听**：`notify` crate 监听已打开文件的元数据变化（debounce 500ms）；磁盘内容变化且文档干净 → 自动重新加载并保留滚动位置；文档脏 → 标题栏黄点 + 非阻断提示"文件已被外部修改"（保存时仍走弹窗三选）。监听器异常/不支持的路径（网络盘部分场景）降级为仅保存前检查，状态栏标注降级原因。
- 崩溃恢复：脏文档在停止输入 2s 后把当前编辑内容（含编码/行尾风格元数据）写入影子文件 `$APP_CONFIG/recovery/<doc_key>.lumelog`；启动时扫描 recovery 目录，逐个询问恢复（恢复后文档仍标脏，需用户保存才落盘）。编辑模式全量存储；只读模式无可恢复内容，不写影子文件。
- 编码不可映射字符（如含 emoji 存 GBK）：用 `encoding_rs` 增量编码器统计 replace 发生次数 N；N>0 时提示"有 N 个字符无法用当前编码表示，将以 U+FFFD 写入，建议改用 UTF-8"，用户确认后执行。

## 5. 编码模块（lumelog-core::encoding）

### 5.1 支持集合

- 显示给用户：`UTF-8`、`UTF-8 BOM`、`UTF-16 LE/BE`、`GBK`、`GB18030`、`BIG5`、`Shift-JIS`、`ISO-8859-1`。
- 实现映射：`GBK / GB2312 → encoding_rs::GBK`（GB2312 实际用 GBK 超集解码，业界惯例）；`GB18030 → encoding_rs::GB18030`。
- UTF-16：BOM 判定字节序；行索引按 `0x0A 0x00`（LE）/`0x00 0x0A`（BE）扫描，且命中位置按文件字节序校验偶数对齐（防在双字节中间误匹配）。v1 范围决定：UTF-16 大文件（>200MB）**仅只读模式**，后端逐块解码后交给虚拟列表（与 §4.3 两档策略一致），不支持编辑。

### 5.2 自动检测顺序

1. BOM（UTF-8/UTF-16）
2. 前 64KB 严格 UTF-8 校验通过 → UTF-8
3. `chardetng`（hint: zh-CN 优先）→ 置信度 ≥ 0.7 采用其结果
4. 兜底 → GB18030（国内工业设备日志默认）【假设：符合目标人群】

检测失败不影响可用性：状态栏常驻当前编码，点击可切换并原地重解码（保留滚动位置，行号不变——行边界由字节索引决定，与编码无关）。

### 5.3 转码保存

"编码菜单 → 另存为 UTF-8/GBK…"：解码字符串 → 目标编码 encode → 写文件。当前文档编码标记不变。

### 5.4 解码错误策略

采用 WHATWG 惯例：非法字节序列替换为 U+FFFD，不中断；每块解码返回 `invalid_count`，大于 0 时状态栏编码名旁标记 "⚠"，提示可能选错编码。

## 6. 搜索与替换（lumelog-core::search）

### 6.1 能力矩阵

| 场景 | 引擎 | 说明 |
|------|------|------|
| 编辑模式搜索替换 | CodeMirror search 插件（前端正则） | 即时高亮、逐个/全部替换；已知限制：JS 正则方言差异（如不支持 `\p{…}` 部分特性）由搜索框旁提示条告知，不做后端校对（v1 砍掉双引擎校对方案，避免复杂性） |
| 大文件只读模式搜索 | 后端分块 `regex` + `memchr` 快速预筛 | 后台执行，进度可取消；块边界重叠 1KB（≥ 正则最大可能匹配长度的一半下限，重叠去重按命中坐标判）；替换入口在只读模式禁用，引导"导出片段后编辑" |

### 6.2 查询预处理

- 用户输入的搜索词先在当前编码下 encode 成字节序列再做字节级正则（这样在 GBK 文件里搜中文直接可用【假设：与 Notepad++ 行为一致】）。
- 正则语法：Rust `regex` crate 语法（前端 CodeMirror 侧做 JS 正则 → Rust 语法的基本转换提示，不支持的特性如反向引用明确报错而非静默）。

### 6.3 命中定位

后端返回 `Match { line, byte_in_line, len, snippet }`（snippet 由后端按当前编码解码后截断±40字符生成）；前端用所在块的 `char_map` 把字节偏移换算为（行号，行内 cu16 偏移）后跳转；块未命中时先取块再换算（命中列表本身不需换算即可展示 snippet）。

### 6.4 行过滤（M3）

视图级过滤：输入包含/正则/取反（`!`）条件（多条件 AND），虚拟列表只渲染匹配行，行号保留原始值。**v1 范围决定：行过滤仅只读模式提供**；编辑模式下 v1 只做高亮标记（§1 定义）不做隐藏行——CodeMirror 隐藏行方案未验证，不留双轨。时序筛选依赖 `Frame.ts`，对无时间戳的源按行号区间退化（§1 已定义）。过滤结果可导出（§7.5 `export_lines`）。

## 7. CAN 模块（lumelog-can）

### 7.1 DBC 解析器（自研，子集语法）

支持语法子集（覆盖 Vector DBC 7.0 常见导出）：

- `VERSION`、`NS_`、`BS_:`、`BU_:`（节点名）
- `BO_ <id> <name>: <dlc> <node>` + `SG_ <name> : <|>|<start>|<len>@<order><sign> (<factor>,<offset>) [<min>|<max>] "<unit>" <receivers>`（注意 `BO_` 行 dlc 与节点间无逗号，以真实 Vector 导出为准）
  - 含复用标记 `M` / `m<n>`（v1 解析存储，解码面板按"当前复用值"过滤显示）
- `VAL_TABLE_`、`VAL_ <msg> <sig> <a> "<b>" …`（枚举值表）
- `CM_` 注释：解析并跳过；`BA_DEF_`/`BA_`：解析并跳过（不实现属性语义）【假设：v1 解码不需要属性】
- 不支持：`BO_TX_BU_` 语义校验、多帧压缩信号组。语法错误报错规则：行号 + 上下文片段，允许"跳过坏行继续解析"模式。

### 7.2 信号解码

- 位提取：Intel（little-endian，`@1`）与 Motorola（`@0`，DBC 斜线位序/Motorola MSB 起始）；两种位序的具体位移算法在实现时**必须以 cantools 参考输出对拍定案**（见 §13.1），不在本 spec 凭记忆写死。有符号按二进制补码；`+`/`-` 无符号/有符号由 `sign` 字符决定。
- 物理值 = raw × factor + offset，factor/offset 为浮点解析（v1 不引入十测定向解析器）。
- 信号值表命中则显示枚举文本，否则显示数值 + 单位。

### 7.3 日志格式解析（v1 两种 + 通用正则）

1. **candump ASCII**：规范形式 `(<time>)? <iface> <CANID>#<DATA>`——长度由 `#` 后 hex 字符数/2 得出（不单独输出 LEN 字段）；RTR 为 `id#R`，FD 帧为 `id##<flags><data>`（flags 含 BRS/ESI 等）；带 `(1.234)` 时间戳前缀的行支持，不带则 ts=None；不支持 error frames 行（`#R` 外的 `id#ErrorFrame` 形态，跳过并计数）。
2. **Vector ASC**：数据帧行形如 `<base> <chan> <ID>x? Rx d <dlc> <b0> <b1> …`（扩展帧 ID 带 `x` 后缀，Remote 帧 `Rx r`）；头部 `date` 行解析时间基准；`boe`/`bcs`/error 等行跳过并计数。真实 ASC 变体多（CANoe 版本差异），语料库测试兼容面见 §13.1。
3. **通用文本行**：用户在"解析配置"里写一条带命名捕获组（`id`、`data`，可选 `ts`）的正则，即可把任意自研日志解码成报文流。【假设：目标用户接受配一次正则】

解析为**逐行流式**：`FrameSource` trait（`fn next_frame(&mut self) -> Option<Frame>`，内部持有 §4 的行流），编辑器与报文表格共享同一 `Frame` 序列，`Frame` 携带 `(line_number, ts, channel, can_id, dlc, data[64], is_fd, flags)`。

### 7.4 CanPanel（报文表格）

- 列：行号、时间戳（可切换相对/绝对、毫秒/微秒）、通道、ID（hex/dec）、DLC、信号展开树、原始 hex。
- 与编辑器双向联动：点行 → 跳源码行；编辑器中选中文本 → "识别为报文"试解码。
- 过滤：ID 范围（`0x100-0x1ff`）、消息名、信号条件（`Speed > 60`）、时间窗口；过滤是视图级的，不改变编辑器行索引。
- 性能：表格虚拟滚动；100 万帧内滚动流畅（P4 同标准）。

### 7.5 导出

- 筛选后的**原始日志行**（按行号从源文件取，保留原编码选项）。
- 解码后的**信号 CSV**（帧 × 信号列宽表，物理值）。
- 解码后的**报文 JSON Lines**（给 HIL 自动化脚本消费）。

## 8. 前端（TypeScript）

- 栈：Vite + TypeScript + CodeMirror 6（编辑模式）+ 自研虚拟列表（只读模式）+ TanStack Table（CanPanel）。
- 【决定】前端不用框架：TypeScript + 原生 DOM 手写组件（UI 规模 <10 个组件），CodeMirror 6 与 TanStack Table 均为框架无关库；不引入 Lit/React。
- 大文件不用 Monaco/CodeMirror 全文档模型的原因：两者的文档模型均要求全量字符串驻留内存，2GB 文件 UTF-16 内部表示直接爆内存；虚拟列表只持有可见 ±缓冲行。
- 主题：内置 浅色/深色（跟随系统）+ 编辑器配色走 CodeMirror theme；主题定义存 JSON（§10）。
- 语法高亮（claude.md 已勾选能力，落到本节）：默认 PlainText；内置 log 风格高亮（时间戳/日志级别/数字/hex 串着色，自定义 Lezer/simple-mode 规则）+ JSON/INI/C 三种常见配置文件；语言从扩展名推断，状态栏可手动切换。
- 标签页：多 Tab；脏标记 = 标题圆点；关闭脏 Tab 弹确认（保存/丢弃/取消）；拖拽排序不做（v1 砍）。
- 快捷键：默认方案（Ctrl+F/Ctrl+Shift+F/Ctrl+G/Ctrl+O/Ctrl+S…）+ 用户覆盖表。

## 9. IPC 契约（Tauri commands）

| 命令 | 方向 | 参数 → 返回 |
|------|------|------|
| `doc_open` | invoke | path → `{ doc_id, size, mtime, encoding, index_progress }` |
| `index_status` | channel 流 | doc_id → `{ indexed_bytes, total_lines? }` 直至完成 |
| `doc_chunk` | invoke | doc_id, line_start, line_count → `{ text, char_map, invalid_count }` |
| `doc_save` | invoke | doc_id, path?, encoding, overwrite_policy → `{ ok }` / 冲突错误 |
| `search_run` | channel 流 | doc_id, query{text\|regex, mode, opts} → `Match` 批量帧 |
| `search_cancel` | invoke | request_id |
| `dbc_import` | invoke | path → `{ dbc_id, messages, errors[] }` |
| `can_parse` | channel 流 | doc_id, dbc_id?, source_kind → `Frame` 批量帧 + 统计 |
| `export_lines` / `export_csv` / `export_jsonl` | invoke | 筛选描述 + 目标路径 → 完成事件（长任务走 channel） |

约定：批量帧 = 每 ≤2ms 或 4096 条聚合一次，防 IPC 风暴；所有长任务带 request_id 可取消；错误统一为 `{ code, message, detail? }` 且 code 可枚举（前端按 code 出文案）。`doc_open` 前置校验：路径不存在/是目录/无读权限/超过 4GB → 对应错误码，弹窗提示不记入最近文件。

## 10. 配置与持久化

- 目录：`$APP_CONFIG/LumeLog/`（Tauri path API）。
- `config.toml`：主题、快捷键覆盖、默认编码兜底、索引/编辑阈值、解析配置（通用正则）。
- `layout.json`：上次会话打开的文件与滚动位置（启动恢复）。
- `recovery/`：崩溃恢复影子文件（§4.4），正常保存/关闭后删除对应项；`config.toml` 可关（`recovery_enabled = false`）。
- 最近文件：`recent.json` 上限 20。

## 11. 错误处理与可观测

1. 三级错误策略（M0 定稿并写进团队公约）：
   - **可恢复**（文件被占用、单行解码失败）：状态栏 + 非阻断提示；
   - **需用户决策**（保存冲突、不可映射字符）：模态；
   - **致命**（索引线程 panic）：log + toast + 该文档降级为"无索引顺序读取"，应用不崩。
2. panic：backend catch_unwind 包裹每个 Tauri command；索引线程崩溃按 §11.1 降级。
3. 日志：`tracing` + rolling file（`$APP_CONFIG/logs/`），级别默认 info，可在设置页调到 debug（复现用户问题时）。
4. 用户报障：设置页"打包诊断信息"（日志 + config + 系统信息 zip 到下载目录）。

## 12. 里程碑与验收

| 里程碑 | 内容 | 验收标准（全部进 CI 或手动清单） |
|--------|------|------|
| M0 脚手架 | Tauri v2 项目、workspace、CI（GitHub Actions 三平台 build+test，产物发 gitee releases；Gitee Go 无公共 Windows/macOS runner）、CLI 冒烟（lumelog-core 自带 bin 打开文件打印行数） | CI 绿；三平台安装包产出 |
| M1 编辑器核心 | 打开/保存/另存、编码检测与切换、编辑模式搜索替换、行索引+只读查看模式 | P1–P5b 达标；1GB 示例日志演示通过 |
| M2 CAN | DBC 子集解析、candump+ASC+通用正则源、CanPanel 联动、行号对齐 | 对 10 万帧真实台架日志解码正确（与 cantools/python-can 抽查比对 ≥99.9%）|
| M3 打磨 | 行过滤/高亮标记/时序筛选、导出三件套、主题/快捷键/layout 恢复、崩溃恢复 + 外部修改监听 | claude.md 勾选清单 v1 项全部 ✅；kill -9 后重启能恢复脏文档；外部改文件后干净文档自动重载 |

**Roadmap（v1 后，架构预留）**：监控 tail 与滚动截断、BLF/MF4、脚本引擎（候选 QuickJS `rquickjs`，暴露 `log.forEachLine/filter/export` 级别 API）、外部插件（Wasm + wit 接口，候选 `Extism` 或自研 host）、多平台打包商店分发、自动更新。

## 13. 测试策略

### 13.1 Rust 单元与对拍测试（`cargo test`）

- 编码：每种编码编解码往返；分块解码跨块多字节字符边界 property test（proptest：任意切分点切开 GBK 流再拼接解码，结果与整体解码逐字符一致）；
- LineIndex：空文件、无尾换行、CRLF、0x0A 风险字节用例（构造含 GB18030 四字节且尾字节 0x39 的样本）；
- DBC：golden 测试（3 份真实风格 DBC → 期望的 message/signal 表）；
- 信号解码：proptest 往返（随机 signal 定义 + 随机帧 → 编码回 bit → 解码一致）；Intel/Motorola 位序各建一组手工向量，与 **cantools（Python）** 参考输出对拍定案（§7.2 的待定算法在此闭环）；
- 日志解析器：candump/ASC 语料库（含畸形行），断言"跳过计数"正确。

### 13.2 基准（criterion）

合成 2GB 日志生成器（固定种子）→ P2/P3/P4/P5 指标；索引缓存命中路径。

### 13.3 前端与手动

IPC 契约用 mock server 测组件逻辑；虚拟列表渲染单测（vitest + happy-dom）。每里程碑发布前过一遍三平台 × 超大文件 × GBK × CAN 演示脚本（写进 docs/test-checklist.md）。

## 14. 开放问题（不阻塞 v1）

- 【待定】macOS 上 GB18030 字体缺失时的中文渲染依赖系统字体，是否内置思源黑体子集？M3 实测后定。
- 【待定】自动更新通道（Tauri updater 需要更新服务器/签名密钥）：v1 发布方式先用 gitee releases 手动更新。

## 15. 假设清单（汇总确认）

| # | 假设 | 影响面 |
|---|------|--------|
| A1 | 编辑阈值 200MB、超过即只读 | 架构、UX |
| A2 | 前端无框架，原生 DOM + CodeMirror 6 | 前端选型 |
| A3 | GB2312 按 GBK 超集处理 | 编码 |
| A4 | 兜底默认编码 GB18030 | 编码 |
| A5 | v1 不做 BA_ 属性语义、不做 UDS/诊断 | CAN |
| A6 | 通用文本日志由用户配一次正则 | CAN |
| A7 | 恢复未保存编辑内容（崩溃恢复影子文件，可关） | 配置 |
| A8 | 搜索在字节层进行、查询串按当前编码编码 | 搜索 |
| A9 | 单文件上限 4GB，超限拒绝打开 | 文档引擎 |
| A10 | 做持续外部修改监听（干净文档自动重载，脏文档提醒） | 保存 |

## 16. 自审记录

| 轮次 | 焦点 | 主要修正 |
|------|------|----------|
| 1 | 占位符/一致性/范围/歧义 | 修正错误章节引用（待验证→§13.1）；砍掉双引擎正则校对；行过滤限定只读模式；candump/ASC 格式描述改准确；CI 平台明确 GitHub Actions；P5 拆为 P5/P5b |
| 2 | 技术细节深查 | line_offsets 改为逐字符 char_map（行粒度无法换算行内字节偏移）；重叠搜索边界策略；UTF-16 扫描偶数对齐；GB18030 误切行影响面澄清；DBC `BO_` 语法去逗号 |
| 3 | 边界与可测性 | 补语法高亮（claude.md 已勾选能力不能漏）；CRLF 保存策略；Windows rename 锁定降级；4GB 上限；空文件/前置校验；§13 编号化使跨节引用成立 |
| 4 | 收敛检查 | 验收范围 P1–P5b；假设清单补 A9/A10；列表格式修正 |
| 5（用户决定） | A7/A10 反转 | 崩溃恢复与外部修改监听纳入 v1：补 §4.4 影子文件/notify 监听设计，§1/§10/§12/§15 同步 |
