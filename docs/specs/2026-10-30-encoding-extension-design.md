# 编码兼容扩展（EUC-KR / EUC-JP / KOI8-R）设计（Plan 16）

对应 `docs/features.md` 待做第 3 条：「编码兼容扩展到更多语种编码（Big5、Shift-JIS、EUC-KR 等）」。

## 1. 背景与现状

`crates/lumelog-core/src/encoding/mod.rs` 的 `EncodingLabel` 目前 9 个变体：
`Utf8 / Utf8Bom / Utf16Le / Utf16Be / Gbk / Gb18030 / Big5 / ShiftJis / Iso88591`。
features.md 那条把 Big5、Shift-JIS 列为缺口是笔误——两者 v1 已交付（解码/编码/检测/前端菜单全链路）。
盘点 `encoding_rs`（WHATWG 编码标准实现）后，真正可补的是 4 个标签：

| 新增 | encoding_rs | chardetng 判定（探针实测） | 备注 |
|---|---|---|---|
| EUC-KR | `EUC_KR` | 带 `cn` hint → GBK；无 hint → EUC-KR（plausible） | 韩文工控设备/ECU 导出日志 |
| EUC-JP | `EUC_JP` | 带 `cn` hint → GBK；无 hint → EUC-JP（plausible） | 日文设备；与 Shift-JIS 同为日文双轨 |
| KOI8-R | `KOI8_R` | chardetng 对俄文一律判 KOI8-U | 俄文系统日志（与 KOI8-U 的差异仅在 8 个制表/乌克兰字母码位，俄文字节完全一致） |
| KOI8-U | `KOI8_U` | 带/无 hint 均 KOI8-U（plausible） | chardetng 的西里尔文检出结果就是它；不加则检出结果映射不到标签、俄文永远兜底 GB18030 |

UTF-32 不做：`encoding_rs`/WHATWG 编码标准不含 UTF-32（检测层在 `detect` 里已注释"UTF-32 not supported in v1"），
真实日志里也见不到。ISO-2022-JP/KR 是带状态转义的邮件编码，日志场景不出现，不引入。

## 1.1 检测的双 pass（本次唯一的检测行为改动）

`detect` 第 3 步的 `cn` hint 会把 EUC-KR/EUC-JP 样本推给 GBK（探针证据见上表，1KB 样本仍如此）。
对策：带 hint 的判定落在 `Gbk | Gb18030` 时，用同一份 head 再跑一次**无 hint** 的 chardetng，
仅当无 hint 结果（plausible）是 `EucJp | EucKr` 才采纳；中文样本无 hint 仍是 GBK（已探针验证），
不会被翻转到日韩文。KOI8 系不受影响（hint 判定直接是 KOI8-U）。
现行的 GB18030 兜底与 §5.2 顺序不变。

## 2. 改动面

### 2.1 Rust（`crates/lumelog-core/src/encoding/mod.rs`）

- `EncodingLabel` 加 `EucJp / EucKr / Koi8R / Koi8U`；
- `as_str()`：`"EUC-JP" / "EUC-KR" / "KOI8-R" / "KOI8-U"`（显示名进菜单与状态栏）；
- `from_label()`：各收一个无连字符别名（`EUCJP / EUCKR / KOI8R / KOI8U`）；
- `encoding()`：映射到对应 `encoding_rs` 编解码器；
- `label_from_encoding()`：chardetng 结果 → 新标签（`KOI8_U → Koi8U`）；
- `detect()`：§1.1 的无 hint 二次确认。

不动：`bom_len`/`has_bom`/`strip_bom`（四个编码都无 BOM，`_ => 0` 已覆盖）；
`IncrementalDecoder`（BOM 缓冲逻辑只对 `bom_len > 0` 生效）；`encode_all` 的 `?` 替换路径
（四个编码都无 U+FFFD 码点，走 legacy 分支即可；KOI8 系的西里尔文全可表示，
日韩文遇 emoji 等同样落到逐字符 `?`）。

### 2.2 前端

- `encoding-menu.ts`：`ENCODINGS` 末尾追加 4 项（display = label，顺序 EUC-JP、EUC-KR、KOI8-R、KOI8-U）；
- `ipc.test.ts`：`encoding_menu_labels_match_spec` 断言 9 → 13 项。

状态栏/切换/重解码链路都从 `ENCODINGS` 派生，无需改。

### 2.3 文档

- `docs/specs/2026-10-24-lumelog-v1-design.md` §5.1 支持集合补 3 项；
- `docs/features.md` 该条移入已交付并修正「Big5、Shift-JIS」笔误、注明 UTF-32 不做的原因；
- `docs/test-checklist.md` §2 冒烟补 EUC-KR/EUC-JP/KOI8-R 样例行。

## 3. 风险

- **检测**：双 pass 只在带 hint 判定为 GBK/GB18030 且无 hint 给出 EUC-JP/EUC-KR 时生效，
  探针已验证中文 GBK/GB18030 样本无 hint 仍是 GBK，不会误翻；`detect_gbk_sample_is_chinese`
  等既有测试继续钉住中文行为。EUC 系与 GBK 互认导致的极端误判仍可能（两者字节区间重叠），
  状态栏 ⚠ 与手动切换是兜底，与现状一致。
- **KOI8-U 显示名**：俄文文件会被检出为 KOI8-U——俄文字节在两个变体里完全一致，解码正确；
  需要严格 KOI8-R 的用户可手动切换。
- **行索引**：四个编码的多字节序列第二字节不含 `0x0A`（EUC 系尾字节 0xA1–0xFE，KOI8 系高位
  0x80–0xFF），不会在双字节中间误切行；spec §4.1 的合并重试策略不受影响。

## 4. 验收

- `cargo test -p lumelog-core encoding::`：四个编码的编解码往返、不可映射 `?` 替换、
  `from_label` 别名、检测样例（EUC-KR/EUC-JP/KOI8-U 检出、中文 GBK 不被翻转）；
- `cargo test --workspace` / `clippy -D warnings` 净；`encoding_proptest.rs` 的 label_pool 补 4 项；
- `npm test`（菜单断言更新）；`npm run build` 通过。
