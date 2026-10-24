# Changelog

按 git log 时间倒序（最新在最上方）。每条写清：改了哪些文件、为什么、验证结果。

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
