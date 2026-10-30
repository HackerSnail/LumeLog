# 功能清单

LumeLog 功能规划与交付记录。`[x]` 已交付，`[ ]` 待做；`↳` 为交付细节（分支 / Plan 编号、实现要点、验收方式）。

## 已交付

- [x] 基础文本编辑、语法高亮、正则搜索替换
- [x] 多编码自动识别与切换
- [x] 流式加载大文件（按需取块虚拟列表，不一次性全载入内存；>200MB 只读）
      ↳ M1 已交付：`DocFile::chunk_at` 分块解码 + 64MB LRU + 前端只读虚拟列表按需取窗；后台流式索引（边索引边刷新）为后续增强
- [x] DBC 文件导入，CAN 报文翻译、信号查看
      ↳ M2 引擎已交付（Plan 2 `feat/m2-can-engine`，纯 Rust `lumelog-can`）：DBC 子集解析 + Intel/Motorola 信号位序解码（cantools golden 对拍）+ candump/ASC/正则三源 + 解码 CLI（10 万帧验收）。Plan 3 `feat/m2-can-panel` 已交付 IPC 命令层与 CanPanel 报文表格 UI：`can_parse` 后台流式解码（`spawn_blocking` + 惰性分块，边解边填、可中途 `can_cancel`）+ `dbc_import` 宽容导入 + `can_decode_snippet` 选中识别；前端 TanStack table-core 虚拟表格 + ID/消息/信号条件/时间窗视图过滤 + 行号双向跳转（列表闪烁 / 编辑器定位）
- [x] 日志行过滤、高亮标记、时序筛选
      ↳ M3 子项目 A 已全部交付：增量1（Plan 4 `feat/m3-filter-highlight`）只读视图多条件 AND 行过滤（包含/排除 × 字面/正则 × 大小写）+ 命中区 cu16 背景高亮 + F3/Shift+F3 跳转 + 实时取消（`filter_run` 异步流式、`doc_lines_at` 按需取子集文本），统一“查询面板”取代旧单框搜索；增量2（Plan 5 `feat/m3-timeline-export`）**时序筛选**：时间戳抽取正则 + 内置格式（ISO8601/epoch s/ms，chrono）按区间过滤，无抽取正则时退化为行号区间。
- [x] 导出筛选后的报文/日志片段
      ↳ 已全部交付：日志片段导出（Plan 5 `feat/m3-timeline-export` `export_lines`，按行号取原始字节流式写、保编码/行尾） + CAN 报文导出（Plan 6 `feat/m3-can-export` `export_csv` 宽表（meta+DBC 信号列、物理值）/`export_jsonl`（每帧对象），后端重解码并套用与 CanPanel 一致的 `frame_matches` 视图过滤，流式可取消）。
- [x] 未保存编辑的崩溃恢复（影子文件 + 启动横幅恢复 + 侧栏开关）
      ↳ M3 子项目 B1（Plan 7 `feat/m3-crash-recovery`）：编辑模式脏文档停输 2s 把全量文本+编码/行尾写入 `$APP_CONFIG/LumeLog/recovery/<FNV(path)>.lumelog`；启动以单列表横幅逐项“恢复/丢弃”（恢复后仍脏、保存才落盘）；保存/关闭/换编码后自动删影；侧栏“崩溃恢复”勾选开关持久化到 `config.toml` 的 `recovery_enabled`（默认开、禁用不删既有影子）。
- [x] 外部修改持续监听（干净自动重载 / 脏黄点+三选 / 删除与降级）
      ↳ M3 子项目 B2（Plan 8 `feat/m3-external-watch`）：`notify` 监听文档父目录（抗 rename）+ 500ms 静默去抖 + `(size,mtime)` 快照抑制自我写入；干净文档自动重载（编辑保光标/滚动；只读保滚动并清查询结果），脏文档黄点 + 非阻断提示条（重新加载/保留），保存冲突**三选**（重新加载/强制覆盖/取消）；删除/改名提示；watcher 失败降级为保存前检查并在状态栏标注；Save-As 后后端重绑 `DocFile` 并迁移监听。
- [x] 自定义主题（浅 / 暗 / Dracula 三主题 + 跟随系统 + localStorage 持久化；无边框自绘标题栏；Ctrl+Shift+P 命令面板）
      ↳ UI 重构 `feat/m-ui-notelume`（参考 NoteLume）已交付：主题 token 体系、三栏外壳（侧栏/检查器/可拖分栏）、命令面板、工具条即标题栏
- [x] 快捷键自定义（内置默认方案 + 用户覆盖；跨设备同步为后续）
      ↳ M3 子项目 C（Plan 9 `feat/m3-keymap`）：命令面板“快捷键设置…”（及侧栏免快捷键入口）可为 13 个动作录制/清除/恢复默认，覆盖存 `config.toml` 的 `[keymap]`；非法/冲突/保留键（F3、Ctrl+F）被拒绝；旧硬编码快捷键改为查表分发，面板 hint 显示生效键。跨设备同步仍为后续。
- [x] Roadmap：tail 跟随与滚动截断（只读）
      ↳ Plan 10 `feat/roadmap-tail-follow`：只读状态栏“跟随尾部”开关，新增字节**增量扩展行索引**（不重建、不整页重载）并自动滚底；上滚暂停 + “回到底部”；截断/轮转（缩小或头签名变化，含 rename 替换）回退全量重载；编辑模式不跟随。
- [x] Roadmap：BLF（Vector 二进制日志）报文源
      ↳ Plan 11 `feat/roadmap-blf`：工具栏“打开 BLF…”直接进 CanPanel；解析 `LOGG` 容器（无压缩/zlib）+ 对象 1/86/100/101；跨容器切分、1ns/10µs 时间；复用过滤/DBC/停止；导出 CSV/JSONL 复用 `export_frames`；无源文本（`lineNumber`=帧序号）。
- [x] Roadmap：MF4（ASAM MDF4）测量数据查看器（增量1 表格 + 增量2 自绘波形）
      ↳ Plan 13 `feat/roadmap-mf4-viewer`：工具栏“测量”/面板“测量数据…”打开 `.mf4`（`falcon_mdf 0.7`，只开 `mmap`；workspace MSRV 1.87→1.89）；独立 dock：元数据行 + 通道搜索/多选（≤32）+ 分页记录表（100/500/1000，时间跳页、列序仅本页，缺失 `—`，不可读列 `⚠`）+ 时间窗 + 流式 CSV 导出（与表格同一单元格映射，可取消、保前缀）；会话上限 4、`mf4_records` 按时间轴分块、其余通道按页取块。
      ↳ Plan 14 `feat/roadmap-mf4-chart`：dock 内「表格 / 波形」tab；`mf4_series` 后端 **min/max 包络**（≤8 序列、≤4000 桶、同一 channel group + 时间主通道、单遍 `aligned_signal_chunks` 流式）；自绘双层 canvas（DPR/ResizeObserver/rAF、滚轮以光标为锚点缩放、拖拽平移、双击复位、hover 十字线 + 原值 tooltip、归一化绘制）；窗口独立，「应用到表格」显式同步；无时间轴禁用波形。
- [x] 语法高亮覆盖常见编程与配置语言（19 种）
      ↳ Plan 15 `feat/syntax-languages` 已交付：`language.ts` 改为注册表（模式 id → 扩展名集合 / 显示名 / 加载器），在原有 Log/INI/JSON/C-C++ 上加 JS/TS、Python、HTML、CSS、XML、Markdown、SQL、Rust、Java（lezer 包）与 YAML/TOML/Shell/Diff/PowerShell（`@codemirror/legacy-modes` 流模式）；`modeIdForExt` 同步推断（AUTO 与未知扩展名都走它），`resolveLanguage` 按需 `import()` 并缓存，语言包不进首屏主 chunk（623.3→506.9KB）；模式 id 与扩展名解耦（`applyMode` 传 id，不再把模式名当扩展名），`setMode` 异步重配带 token 防连切覆盖，destroy 后回包不 dispatch；状态栏下拉显示「C/C++」「TypeScript」等显示名、回传仍是模式 id。大文件只读路径无 CodeMirror，仍不高亮。
- [x] 编码兼容：EUC-JP、EUC-KR、KOI8-R、KOI8-U（原条目所列 Big5、Shift-JIS v1 已交付，属笔误）
      ↳ Plan 16 `feat/encoding-extension`：`EncodingLabel` 加 4 变体（`as_str`/`from_label` 含 `EUCJP` 等别名/`encoding()` 映射），解码、编码（不可映射仍单字节 `?`）、前端菜单 13 项全链路；检测双 pass——带 `cn` hint 判成 GBK/GB18030 时无 hint 复核、仅采纳 EUC-JP/EUC-KR（探针实测中文样本不受影响），俄文按 chardetng 结果检出 KOI8-U（与 KOI8-R 俄文字节一致）。UTF-32 无 WHATWG/encoding_rs 编码，不做。
- [x] 脚本引擎（QuickJS 逐行变换）
      ↳ Plan 12 `feat/roadmap-script`：工具栏/面板“脚本…”开启脚本管理器（`$APP_CONFIG/LumeLog/scripts/*.js`，列表/新建模板/保存/删除 + 控制台）；`onLine(line, ctx)` 对当前文档**逐行流式**变换（返回 string 保留/改写、null 丢弃），输出 UTF-8/LF 新文件；`rquickjs 0.14` 沙箱（无 fs/net/require，64MiB 内存/1MiB 栈/300s 超时/4GiB 输出上限、可取消），workspace MSRV 1.79→1.87。

## 待做

- [ ] 插件接口，支持用户扩展解析器：冷门编程语言的高亮/解析、冷门日志与报文格式，都应能以插件形式补入，不改主程序
      ↳ 增量1 已交付（Plan 17 `feat/plugin-syntax`，spec `docs/specs/2026-10-30-plugin-syntax-design.md`）：声明式语法高亮插件——`$APP_CONFIG/LumeLog/plugins/*.json` 一个文件一个语法（`id/name/extensions/lineComment/keywords/rules`，规则 `regex + tag`，tag 限 className/keyword/number/string/comment/propertyName/operator 七个），Rust `plugins.rs` 校验加载（坏文件逐条报错不阻塞、id 按文件名序去重、`deny_unknown_fields`），前端编译为 StreamLanguage（正则 JS 方言校验在前端，坏规则跳过并回报）；状态栏「语法」下拉出现插件模式（id `plugin:<id>`），AUTO 按扩展名匹配（内置优先）；命令面板「重载语法插件」。
      ↳ 增量2 已交付（Plan 18 `feat/plugin-parser`，spec `docs/specs/2026-10-30-plugin-parser-design.md`）：QuickJS 逐行解析插件——`plugins/*.js` 定义 `var plugin = { id, name, extensions, columns, parseLine(line, ctx) }`，返回对象（列 key→值）或 null 跳过；rquickjs 沙箱（同脚本引擎 64MiB/1MiB 栈/300s/可取消，元数据加载 5s）；工具栏「解析」开 dock（与报文/测量互斥），按文档扩展名预选插件，512 行/批流式进虚拟表格（`parserpanel.ts`，行号列 + 插件列），点行跳源行，插件 console 输出进 dock 控制台（200 行上限）；单元格字符串化截断 4096 字符。无过滤/导出（后续搬 CanPanel 能力）；Wasm host 为增量3，待做。
- [ ] 常见文档格式查看与导出（PDF、PPT、docx/xlsx 等 Office 格式）
      ↳ 增量1 已交付（Plan 19 `feat/doc-viewer`，spec `docs/specs/2026-10-30-doc-viewer-design.md`）：工具栏「查看」打开 xlsx/xlsm（`calamine` 表格：sheet 下拉、窗口加载、CSV 导出）与 docx（zip+quick-xml 提取正文段落、TXT 导出）；复用解析插件的虚拟表格（`ParserPanel`）。PDF 推迟（查看需 pdfium 级渲染引擎，原生库体积与分发代价大）；PPT 推迟（版式复杂、文本提取价值低）；xls/ods 推迟（calamine 支持但测试面翻倍）。
