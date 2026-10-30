//! Encoding detection, decoding, and encoding for the document engine.
//! Display labels are pinned by spec §5.1; detection order by spec §5.2;
//! decode error policy (U+FFFD replace) by spec §5.4.

use encoding_rs::Encoding;

/// User-facing encoding identities (spec §5.1). `GB2312` is intentionally
/// absent — its input is decoded as `GBK` (superset), see `from_label`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingLabel {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Gbk,
    Gb18030,
    Big5,
    ShiftJis,
    Iso88591,
    EucJp,
    EucKr,
    Koi8R,
    Koi8U,
}

impl EncodingLabel {
    /// Fixed display name shown in the status bar / menu (Global Constraints).
    pub fn as_str(self) -> &'static str {
        match self {
            EncodingLabel::Utf8 => "UTF-8",
            EncodingLabel::Utf8Bom => "UTF-8 BOM",
            EncodingLabel::Utf16Le => "UTF-16 LE",
            EncodingLabel::Utf16Be => "UTF-16 BE",
            EncodingLabel::Gbk => "GBK",
            EncodingLabel::Gb18030 => "GB18030",
            EncodingLabel::Big5 => "BIG5",
            EncodingLabel::ShiftJis => "Shift-JIS",
            EncodingLabel::Iso88591 => "ISO-8859-1",
            EncodingLabel::EucJp => "EUC-JP",
            EncodingLabel::EucKr => "EUC-KR",
            EncodingLabel::Koi8R => "KOI8-R",
            EncodingLabel::Koi8U => "KOI8-U",
        }
    }

    /// Parse a display label. `GB2312` maps to `Gbk` (spec §5.1).
    pub fn from_label(s: &str) -> Option<EncodingLabel> {
        match s {
            "UTF-8" => Some(EncodingLabel::Utf8),
            "UTF-8 BOM" => Some(EncodingLabel::Utf8Bom),
            "UTF-16 LE" => Some(EncodingLabel::Utf16Le),
            "UTF-16 BE" => Some(EncodingLabel::Utf16Be),
            "GBK" | "GB2312" | "GB_2312-80" => Some(EncodingLabel::Gbk),
            "GB18030" => Some(EncodingLabel::Gb18030),
            "BIG5" => Some(EncodingLabel::Big5),
            "Shift-JIS" | "SHIFT_JIS" | "CP932" => Some(EncodingLabel::ShiftJis),
            "ISO-8859-1" | "windows-1252" | "WINDOWS-1252" => Some(EncodingLabel::Iso88591),
            "EUC-JP" | "EUCJP" => Some(EncodingLabel::EucJp),
            "EUC-KR" | "EUCKR" => Some(EncodingLabel::EucKr),
            "KOI8-R" | "KOI8R" => Some(EncodingLabel::Koi8R),
            "KOI8-U" | "KOI8U" => Some(EncodingLabel::Koi8U),
            _ => None,
        }
    }

    /// The `encoding_rs` codec backing this label.
    fn encoding(self) -> &'static Encoding {
        match self {
            EncodingLabel::Utf8 | EncodingLabel::Utf8Bom => encoding_rs::UTF_8,
            EncodingLabel::Utf16Le => encoding_rs::UTF_16LE,
            EncodingLabel::Utf16Be => encoding_rs::UTF_16BE,
            EncodingLabel::Gbk => encoding_rs::GBK,
            EncodingLabel::Gb18030 => encoding_rs::GB18030,
            EncodingLabel::Big5 => encoding_rs::BIG5,
            EncodingLabel::ShiftJis => encoding_rs::SHIFT_JIS,
            EncodingLabel::Iso88591 => encoding_rs::WINDOWS_1252,
            EncodingLabel::EucJp => encoding_rs::EUC_JP,
            EncodingLabel::EucKr => encoding_rs::EUC_KR,
            EncodingLabel::Koi8R => encoding_rs::KOI8_R,
            EncodingLabel::Koi8U => encoding_rs::KOI8_U,
        }
    }
}

/// Byte-order-mark length for the label (0 if none expected).
pub fn bom_len(label: EncodingLabel) -> usize {
    match label {
        EncodingLabel::Utf8Bom => 3,
        EncodingLabel::Utf16Le | EncodingLabel::Utf16Be => 2,
        _ => 0,
    }
}

/// How many leading bytes `decode_all` would strip as a BOM from `bytes`.
pub fn leading_bom_len(label: EncodingLabel, bytes: &[u8]) -> usize {
    if has_bom(label, bytes) {
        bom_len(label)
    } else {
        0
    }
}

/// Strip the leading BOM matching `label`, if present.
fn strip_bom(label: EncodingLabel, bytes: &[u8]) -> &[u8] {
    let n = bom_len(label);
    if bytes.len() >= n && has_bom(label, bytes) {
        &bytes[n..]
    } else {
        bytes
    }
}

pub(crate) fn has_bom(label: EncodingLabel, bytes: &[u8]) -> bool {
    match label {
        EncodingLabel::Utf8Bom => bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
        EncodingLabel::Utf16Le => bytes.starts_with(&[0xFF, 0xFE]),
        EncodingLabel::Utf16Be => bytes.starts_with(&[0xFE, 0xFF]),
        _ => false,
    }
}

/// Result of decoding a whole buffer.
pub struct Decoded {
    pub text: String,
    /// Number of U+FFFD replacement characters produced (spec §5.4 heuristic).
    pub invalid_count: usize,
}

/// Detect the encoding of a file from its leading bytes (spec §5.2).
/// `head` is the first up-to-64KB of the file.
pub fn detect(head: &[u8]) -> EncodingLabel {
    // 1. BOM
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return EncodingLabel::Utf8Bom;
    }
    if head.starts_with(&[0xFF, 0xFE]) {
        // UTF-16LE BOM; distinguish UTF-32LE (0xFF 0xFE 0x00 0x00) not supported in v1.
        return EncodingLabel::Utf16Le;
    }
    if head.starts_with(&[0xFE, 0xFF]) {
        return EncodingLabel::Utf16Be;
    }
    // 2. strict UTF-8 over the head. A truncated sequence at the very end
    //    (error_len() == None) is just the 64KB window cutting a char in half —
    //    still UTF-8, not a real invalid byte.
    match std::str::from_utf8(head) {
        Ok(_) => return EncodingLabel::Utf8,
        Err(e) if e.error_len().is_none() => return EncodingLabel::Utf8,
        _ => {}
    }
    // 3. chardetng, hinting a Chinese TLD (spec §5.2). chardetng 0.1 returns
    //    (encoding, plausible): `plausible` means it beat at least one other
    //    candidate — the crate's proxy for confidence. We require it. allow_utf8
    //    = true so a head truncated mid-char that failed step 2 can still land UTF-8.
    let mut det = chardetng::EncodingDetector::new();
    det.feed(head, true);
    let (enc, plausible) = det.guess_assess(Some(b"cn"), true);
    if plausible {
        if let Some(label) = label_from_encoding(enc) {
            // The `cn` hint pushes double-byte EUC families toward GBK: a 1KB
            // Korean/Japanese sample already reads as plausible GBK. Re-ask
            // WITHOUT the hint and accept only the verdicts that pass cannot
            // produce for real Chinese text (probed: GBK/GB18030 samples stay
            // GBK without a hint). Everything else keeps the hinted answer.
            if matches!(label, EncodingLabel::Gbk | EncodingLabel::Gb18030) {
                let mut unbiased = chardetng::EncodingDetector::new();
                unbiased.feed(head, true);
                let (enc2, plausible2) = unbiased.guess_assess(None, true);
                if plausible2 {
                    if let Some(retry) = label_from_encoding(enc2) {
                        if matches!(retry, EncodingLabel::EucJp | EncodingLabel::EucKr) {
                            return retry;
                        }
                    }
                }
            }
            return label;
        }
    }
    // 4. fallback
    EncodingLabel::Gb18030
}

/// Map an `encoding_rs::Encoding` (from chardetng) to our label set.
fn label_from_encoding(enc: &'static Encoding) -> Option<EncodingLabel> {
    if enc == encoding_rs::UTF_8 {
        Some(EncodingLabel::Utf8)
    } else if enc == encoding_rs::UTF_16LE {
        Some(EncodingLabel::Utf16Le)
    } else if enc == encoding_rs::UTF_16BE {
        Some(EncodingLabel::Utf16Be)
    } else if enc == encoding_rs::GBK {
        Some(EncodingLabel::Gbk)
    } else if enc == encoding_rs::GB18030 {
        Some(EncodingLabel::Gb18030)
    } else if enc == encoding_rs::BIG5 {
        Some(EncodingLabel::Big5)
    } else if enc == encoding_rs::SHIFT_JIS {
        Some(EncodingLabel::ShiftJis)
    } else if enc == encoding_rs::WINDOWS_1252 {
        Some(EncodingLabel::Iso88591)
    } else if enc == encoding_rs::EUC_JP {
        Some(EncodingLabel::EucJp)
    } else if enc == encoding_rs::EUC_KR {
        Some(EncodingLabel::EucKr)
    } else if enc == encoding_rs::KOI8_R {
        Some(EncodingLabel::Koi8R)
    } else if enc == encoding_rs::KOI8_U {
        // chardetng reports KOI8-U for Russian text too — Russian letters sit at
        // identical code points in both variants, so the decode is correct
        // either way (see spec §1.1 / §3).
        Some(EncodingLabel::Koi8U)
    } else {
        None
    }
}

/// Decode an entire buffer in one shot. `invalid_count` is the number of
/// U+FFFD in the output (spec §5.4 heuristic — a literal U+FFFD in valid text
/// also counts; documented approximation, kept consistent with `feed`).
pub fn decode_all(label: EncodingLabel, bytes: &[u8]) -> Decoded {
    let body = strip_bom(label, bytes);
    let (cow, _had_errors) = label.encoding().decode_without_bom_handling(body);
    let invalid_count = cow.matches('\u{FFFD}').count();
    Decoded {
        text: cow.into_owned(),
        invalid_count,
    }
}

/// Decode a byte slice taken from the *middle* of a stream: never strips a BOM
/// (a `EF BB BF` / `FF FE` sequence inside a chunk is content, e.g. a stray
/// U+FEFF, not a file mark). Used by the chunk reader for any line whose byte
/// span does not start at file offset 0.
pub fn decode_region(label: EncodingLabel, bytes: &[u8]) -> Decoded {
    let (cow, _had_errors) = label.encoding().decode_without_bom_handling(bytes);
    let invalid_count = cow.matches('\u{FFFD}').count();
    Decoded {
        text: cow.into_owned(),
        invalid_count,
    }
}

/// One segment produced by `IncrementalDecoder::feed`.
pub struct DecodeSegment {
    pub text: String,
    pub invalid_count: usize,
}

/// Stateful decoder that yields output identical to `decode_all` regardless of
/// where the input byte stream is split (pinned by proptest). One instance
/// decodes exactly one stream: `feed` any number of times, then `finish` (which
/// consumes the decoder — encoding_rs forbids reusing a flushed decoder).
pub struct IncrementalDecoder {
    label: EncodingLabel,
    decoder: encoding_rs::Decoder,
    /// Leading bytes buffered until we have enough to decide whether a BOM is
    /// present. Cleared once the BOM decision is made (`bom_done`).
    head: Vec<u8>,
    bom_done: bool,
}

impl IncrementalDecoder {
    /// Construct for a whole-stream decode starting at byte 0 (BOM-aware).
    pub fn new(label: EncodingLabel) -> Result<Self, crate::error::CoreError> {
        let decoder = label.encoding().new_decoder_without_bom_handling();
        Ok(Self {
            label,
            decoder,
            head: Vec::new(),
            // Labels without a BOM concept need no buffering.
            bom_done: bom_len(label) == 0,
        })
    }

    pub fn feed(&mut self, bytes: &[u8]) -> DecodeSegment {
        if self.bom_done {
            return self.decode_body(bytes, false);
        }
        // Accumulate the head until we can tell whether a BOM is present.
        self.head.extend_from_slice(bytes);
        if self.head.len() < bom_len(self.label) {
            return DecodeSegment {
                text: String::new(),
                invalid_count: 0,
            };
        }
        let body = self.take_bom_stripped_head();
        self.decode_body(&body, false)
    }

    /// Flush trailing incomplete sequences (emits U+FFFD for a truncated
    /// multi-byte tail) and return the flushed text + replacement count.
    /// Consumes `self`: a flushed `encoding_rs::Decoder` must not be reused.
    pub fn finish(mut self) -> DecodeSegment {
        // If the stream ended before we could size the BOM, decide now.
        let pending = if self.bom_done {
            Vec::new()
        } else {
            self.take_bom_stripped_head()
        };
        let mut seg = self.decode_body(&pending, false);
        let tail = self.decode_body(&[], true);
        seg.text.push_str(&tail.text);
        seg.invalid_count += tail.invalid_count;
        seg
    }

    /// Uses the accumulated `head` exactly once: strips a matching BOM prefix
    /// and returns the remaining bytes as an owned buffer.
    fn take_bom_stripped_head(&mut self) -> Vec<u8> {
        let n = bom_len(self.label);
        let body = if has_bom(self.label, &self.head) {
            self.head[n..].to_vec()
        } else {
            std::mem::take(&mut self.head)
        };
        self.head.clear();
        self.bom_done = true;
        body
    }

    /// Decode `bytes` (already BOM-free) into a fresh segment. `last` flushes
    /// the decoder at end-of-stream. Loops until the input is fully consumed.
    fn decode_body(&mut self, bytes: &[u8], last: bool) -> DecodeSegment {
        let mut out = String::new();
        let mut invalid = 0usize;
        let mut src = bytes;
        loop {
            out.reserve(src.len() * 3 + 16);
            let before = out.len();
            let (result, read, _replaced) = self.decoder.decode_to_string(src, &mut out, last);
            invalid += out[before..].matches('\u{FFFD}').count();
            src = &src[read..];
            // InputEmpty = all input consumed. OutputFull = dst filled; loop
            // reserves more and drains the rest of `src` (read > 0 guarantees
            // forward progress there).
            if matches!(result, encoding_rs::CoderResult::InputEmpty) {
                break;
            }
        }
        DecodeSegment {
            text: out,
            invalid_count: invalid,
        }
    }
}

/// Result of encoding text to bytes.
pub struct Encoded {
    pub bytes: Vec<u8>,
    /// Number of characters that could not be represented (spec §4.4 confirm flow).
    pub replace_count: usize,
}

/// Encode `text` to `label`'s bytes. UTF variants are laid out by hand so byte
/// order / BOM are deterministic; legacy codecs delegate to `encoding_rs`.
pub fn encode_all(label: EncodingLabel, text: &str) -> Encoded {
    match label {
        EncodingLabel::Utf8 => Encoded {
            bytes: text.as_bytes().to_vec(),
            replace_count: 0,
        },
        EncodingLabel::Utf8Bom => {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(text.as_bytes());
            Encoded {
                bytes,
                replace_count: 0,
            }
        }
        EncodingLabel::Utf16Le | EncodingLabel::Utf16Be => {
            let le = label == EncodingLabel::Utf16Le;
            let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
            bytes.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
            for unit in text.encode_utf16() {
                if le {
                    bytes.extend_from_slice(&unit.to_le_bytes());
                } else {
                    bytes.extend_from_slice(&unit.to_be_bytes());
                }
            }
            Encoded {
                bytes,
                replace_count: 0,
            }
        }
        other => {
            let enc = other.encoding();
            // Fast path: a single bulk encode. When every code point is
            // representable (the overwhelmingly common case) the result is byte-
            // identical to the per-code-point walk below but allocates once, not
            // once per non-ASCII char — which matters because the target workload
            // is multi-megabyte Chinese logs.
            let (cow, _actual, had_errors) = enc.encode(text);
            if !had_errors {
                return Encoded {
                    bytes: cow.into_owned(),
                    replace_count: 0,
                };
            }
            // Only when something is unrepresentable do we re-walk code point by
            // code point. `encoding_rs`'s bulk lossy `encode` writes unmappable
            // characters as an HTML decimal numeric reference (`&#128169;`) — 9
            // bytes that silently rewrite the line, change its length and break
            // round-tripping. Spec §4.4 instead asks for a single replacement
            // marker; the codecs that actually hit this (GBK/BIG5/Shift-JIS/
            // ISO-8859-1) have no U+FFFD code point, so the only marker that
            // stays exactly one byte and re-decodes cleanly is `?`. (GB18030
            // covers all Unicode and only reaches here for U+E5E5, which has no
            // round-trip byte.)
            let mut bytes = Vec::with_capacity(text.len());
            let mut replace_count = 0usize;
            let mut scratch = [0u8; 4];
            for c in text.chars() {
                let (out, _n, had_err) = enc.encode(c.encode_utf8(&mut scratch));
                if had_err {
                    replace_count += 1;
                    bytes.push(b'?');
                } else {
                    bytes.extend_from_slice(&out);
                }
            }
            Encoded { bytes, replace_count }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_bom_variants() {
        assert_eq!(
            detect(&[0xEF, 0xBB, 0xBF, b'h', b'i']),
            EncodingLabel::Utf8Bom
        );
        assert_eq!(detect(&[0xFF, 0xFE, 0x00, 0x00]), EncodingLabel::Utf16Le);
        assert_eq!(detect(&[0xFE, 0xFF, 0x00, 0x00]), EncodingLabel::Utf16Be);
    }

    #[test]
    fn detect_utf8_strict_beats_chardetng() {
        // "中文" in UTF-8 is valid UTF-8 and would otherwise confuse a
        // statistical detector; step 2 must win.
        let head = "台架 HIL 测试 OK\n".as_bytes();
        assert_eq!(detect(head), EncodingLabel::Utf8);
    }

    #[test]
    fn detect_gbk_sample_is_chinese() {
        let (_, _, err) = encoding_rs::GBK.encode("动力系统控制台台架测试数据记录");
        assert!(!err);
        let gbk = encoding_rs::GBK.encode("动力系统控制台台架测试数据记录").0;
        let label = detect(&gbk);
        assert!(
            matches!(label, EncodingLabel::Gbk | EncodingLabel::Gb18030),
            "expected a Chinese label, got {label:?}"
        );
    }

    #[test]
    fn detect_falls_back_when_result_not_in_label_set() {
        // Cyrillic in WINDOWS_1251 is valid & plausible for chardetng but NOT
        // in our supported label map -> detection must fall through to GB18030.
        let cyr = encoding_rs::WINDOWS_1251.encode("Привет мир, это тест").0;
        assert_eq!(detect(&cyr), EncodingLabel::Gb18030);
    }

    #[test]
    fn from_label_gb2312_maps_to_gbk() {
        assert_eq!(
            EncodingLabel::from_label("GB2312"),
            Some(EncodingLabel::Gbk)
        );
        assert_eq!(EncodingLabel::Gbk.as_str(), "GBK");
    }

    #[test]
    fn from_label_accepts_new_labels_and_aliases() {
        for (name, label) in [
            ("EUC-JP", EncodingLabel::EucJp),
            ("EUCJP", EncodingLabel::EucJp),
            ("EUC-KR", EncodingLabel::EucKr),
            ("EUCKR", EncodingLabel::EucKr),
            ("KOI8-R", EncodingLabel::Koi8R),
            ("KOI8R", EncodingLabel::Koi8R),
            ("KOI8-U", EncodingLabel::Koi8U),
            ("KOI8U", EncodingLabel::Koi8U),
        ] {
            assert_eq!(EncodingLabel::from_label(name), Some(label), "{name}");
        }
        assert_eq!(EncodingLabel::EucJp.as_str(), "EUC-JP");
        assert_eq!(EncodingLabel::EucKr.as_str(), "EUC-KR");
        assert_eq!(EncodingLabel::Koi8R.as_str(), "KOI8-R");
        assert_eq!(EncodingLabel::Koi8U.as_str(), "KOI8-U");
    }

    #[test]
    fn eucjp_euckr_koi8r_roundtrip() {
        let cases = [
            (EncodingLabel::EucJp, "システム起動ログ: 温度センサー値を記録"),
            (EncodingLabel::EucKr, "시스템 로그 테스트: 모터 토크 값 기록"),
            (EncodingLabel::Koi8R, "Запуск системы: журнал тестирования"),
            (EncodingLabel::Koi8U, "Запуск системы: журнал тестирования"),
        ];
        for (label, text) in cases {
            let enc = encode_all(label, text);
            assert_eq!(enc.replace_count, 0, "{label:?}");
            let dec = decode_all(label, &enc.bytes);
            assert_eq!(dec.text, text, "{label:?}");
            assert_eq!(dec.invalid_count, 0, "{label:?}");
        }
    }

    #[test]
    fn korean_emoji_is_unmappable() {
        // Same contract as `gbk_emoji_is_unmappable`: EUC-KR has no U+FFFD code
        // point, so the marker is a single `?` — no numeric reference, and the
        // placeholder must survive a re-decode.
        let enc = encode_all(EncodingLabel::EucKr, "ok💩");
        assert_eq!(enc.replace_count, 1);
        let dec = decode_all(EncodingLabel::EucKr, &enc.bytes);
        assert_eq!(dec.text, "ok?");
        assert!(!enc.bytes.windows(2).any(|w| w == b"&#"));
    }

    #[test]
    fn detect_euckr_sample() {
        let line = "시스템 로그 테스트: 모터 토크 값 기록 완료 2026-10-30\n";
        let text = line.repeat(20);
        let raw = encoding_rs::EUC_KR.encode(&text).0;
        assert_eq!(detect(&raw), EncodingLabel::EucKr);
    }

    #[test]
    fn detect_eucjp_sample() {
        let line = "システム起動ログ: 温度センサー値を記録しました 2026-10-30\n";
        let text = line.repeat(20);
        let raw = encoding_rs::EUC_JP.encode(&text).0;
        assert_eq!(detect(&raw), EncodingLabel::EucJp);
    }

    #[test]
    fn detect_cyrillic_sample_is_koi8u() {
        // chardetng reports KOI8-U for Russian text (both hint passes); Russian
        // letters share code points with KOI8-R so the decode is correct. The
        // strict KOI8-R label stays available for manual selection.
        let line = "Запуск системы: журнал тестирования датчиков 2026-10-30\n";
        let text = line.repeat(20);
        let raw = encoding_rs::KOI8_R.encode(&text).0;
        assert_eq!(detect(&raw), EncodingLabel::Koi8U);
    }

    #[test]
    fn gbk_roundtrip() {
        let text = "台架HIL测试";
        let enc = encode_all(EncodingLabel::Gbk, text);
        assert_eq!(enc.replace_count, 0);
        let dec = decode_all(EncodingLabel::Gbk, &enc.bytes);
        assert_eq!(dec.text, text);
        assert_eq!(dec.invalid_count, 0);
    }

    #[test]
    fn gbk_emoji_is_unmappable() {
        let enc = encode_all(EncodingLabel::Gbk, "ok💩");
        assert_eq!(enc.replace_count, 1, "emoji cannot be represented in GBK");
        // Spec §4.4 promises a replacement char, not encoding_rs' HTML numeric
        // reference (`&#128169;`). GBK has no U+FFFD code point, so the marker is
        // `?`; crucially the output must not contain a numeric ref and must
        // re-decode to a single placeholder char (line length preserved).
        let dec = decode_all(EncodingLabel::Gbk, &enc.bytes);
        assert_eq!(dec.text, "ok?", "unmappable collapses to a single placeholder");
        assert!(!enc.bytes.windows(2).any(|w| w == b"&#"), "no numeric ref");
    }

    #[test]
    fn utf16le_roundtrip_with_bom() {
        let enc = encode_all(EncodingLabel::Utf16Le, "ab线");
        assert!(enc.bytes.starts_with(&[0xFF, 0xFE]));
        let dec = decode_all(EncodingLabel::Utf16Le, &enc.bytes);
        assert_eq!(dec.text, "ab线");
        assert_eq!(dec.invalid_count, 0);
    }

    #[test]
    fn utf16be_roundtrip_and_detect() {
        let enc = encode_all(EncodingLabel::Utf16Be, "ab线");
        assert!(enc.bytes.starts_with(&[0xFE, 0xFF]), "BE BOM first");
        assert_eq!(detect(&enc.bytes), EncodingLabel::Utf16Be);
        let dec = decode_all(EncodingLabel::Utf16Be, &enc.bytes);
        assert_eq!(dec.text, "ab线");
        assert_eq!(dec.invalid_count, 0);
    }

    // Deterministic lock-in for the two Critical streaming fixes: BOM split
    // across sub-`bom_len` chunks (Critical-3) and flushed tail text returned
    // by `finish` (Critical-1). proptest covers these only probabilistically.
    #[test]
    fn bom_split_across_tiny_chunks_and_finish_flush() {
        // UTF-8 BOM + "hi" + a literal U+FFFD.
        let raw = encode_all(EncodingLabel::Utf8Bom, "hi\u{FFFD}").bytes;
        let mut d = IncrementalDecoder::new(EncodingLabel::Utf8Bom).unwrap();
        let mut t = String::new();
        // Feed the 3 BOM bytes one at a time (each chunk < bom_len), then rest.
        for b in raw[..3].iter() {
            t.push_str(&d.feed(std::slice::from_ref(b)).text);
        }
        t.push_str(&d.feed(&raw[3..]).text);
        t.push_str(&d.finish().text);
        let whole = decode_all(EncodingLabel::Utf8Bom, &raw);
        assert_eq!(t, whole.text);
        assert_eq!(whole.invalid_count, 1, "literal U+FFFD counted once");
    }

    #[test]
    fn finish_returns_flushed_tail_text() {
        // A GBK stream truncated mid-character: the trailing lead byte must
        // surface as U+FFFD via `finish`, not be silently dropped.
        let mut raw = encode_all(EncodingLabel::Gbk, "数据线").bytes;
        raw.pop(); // drop one trail byte -> dangling lead byte held internally
        let mut d = IncrementalDecoder::new(EncodingLabel::Gbk).unwrap();
        let mut t = String::new();
        let mut invalid = 0usize;
        // Feed the whole (truncated) stream: the decoder holds the dangling
        // lead byte internally; `finish` flushes it into a U+FFFD.
        let seg = d.feed(&raw);
        t.push_str(&seg.text);
        invalid += seg.invalid_count;
        let tail = d.finish();
        t.push_str(&tail.text);
        invalid += tail.invalid_count;
        let whole = decode_all(EncodingLabel::Gbk, &raw);
        assert_eq!(t, whole.text, "flushed tail text must not be dropped");
        assert_eq!(invalid, whole.invalid_count, "flushed replacement must be counted");
        assert!(whole.invalid_count >= 1, "truncated char should yield a U+FFFD");
    }
}
