//! Document viewers (doc-viewer increment 1 — spec
//! `docs/specs/2026-10-30-doc-viewer-design.md`): xlsx sheets as tables
//! (`calamine`) and docx body text (zip + `quick-xml` over `w:p`/`w:t`).
//! Sessions mirror `Mf4Sessions`: bounded map, windowed row reads, close
//! idempotent. The frontend reuses the parser table, so rows are emitted as
//! `{ line, cells }` — the exact `ParserRow` shape.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;

use calamine::{Data, Reader, Xlsx};
use serde::Serialize;

use crate::dto::IpcError;

/// Upper bounds (spec §2) — a viewer is a convenience, not a data pipeline.
pub(crate) const MAX_VIEWER_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_XML_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PARAGRAPHS: usize = 200_000;
const MAX_PARA_TEXT_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_COLUMNS: usize = 64;
pub(crate) const MAX_WINDOW: u32 = 1000;
const MAX_EXPORT_ROWS: u64 = 1_048_576;
const SESSION_CAP: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewerKind {
    Xlsx,
    Docx,
    /// Text-extraction view (doc-viewer increment 1 of the PDF proposal):
    /// no layout, lines land in the same single-column path as docx.
    Pdf,
}

impl ViewerKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ViewerKind::Xlsx => "xlsx",
            ViewerKind::Docx => "docx",
            ViewerKind::Pdf => "pdf",
        }
    }
}

struct ViewerSession {
    kind: ViewerKind,
    workbook: Option<Xlsx<BufReader<File>>>,
    sheets: Vec<String>,
    /// docx paragraphs (extracted once at open; xlsx keeps None).
    docx_lines: Option<Vec<String>>,
    docx_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerInfoJson {
    pub id: u64,
    /// `"xlsx" | "docx"`.
    pub kind: String,
    pub path: String,
    pub sheets: Vec<String>,
    /// docx: paragraph count (xlsx: null).
    pub paragraphs: Option<u64>,
    /// docx: true when the extraction hit the paragraph/text cap.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerRowJson {
    /// 1-based row number as shown in the table (spreadsheet row / paragraph).
    pub line: u64,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerPageJson {
    pub total: u64,
    /// Header letters ("A", "B", …) for xlsx; `["内容"]` for docx.
    pub columns: Vec<String>,
    pub rows: Vec<ViewerRowJson>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerExportStats {
    pub written: u64,
}

#[derive(Default)]
pub(crate) struct ViewerSessions {
    sessions: HashMap<u64, ViewerSession>,
    next: u64,
}

impl ViewerSessions {
    pub(crate) fn open(&mut self, path: &Path) -> Result<ViewerInfoJson, IpcError> {
        if self.sessions.len() >= SESSION_CAP {
            return Err(IpcError::new(
                "invalid_argument",
                format!("查看会话已达 {SESSION_CAP} 个上限，请先关闭一个"),
            ));
        }
        let meta = std::fs::metadata(path).map_err(|e| IpcError::new("io", format!("stat: {e}")))?;
        if meta.len() > MAX_VIEWER_FILE_BYTES {
            return Err(IpcError::new(
                "invalid_argument",
                format!("文件超过 {MAX_VIEWER_FILE_BYTES} 字节上限"),
            ));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let session = match ext.as_str() {
            "xlsx" | "xlsm" => self.open_xlsx(path)?,
            "docx" => self.open_docx(path)?,
            "pdf" => self.open_pdf(path)?,
            _ => {
                return Err(IpcError::new(
                    "viewer_unsupported",
                    format!("不支持的查看格式：{ext}（支持 xlsx/xlsm/docx）"),
                ))
            }
        };
        let info = ViewerInfoJson {
            id: self.next,
            kind: session.kind.as_str().into(),
            path: path.to_string_lossy().into_owned(),
            sheets: session.sheets.clone(),
            paragraphs: session.docx_lines.as_ref().map(|l| l.len() as u64),
            truncated: session.docx_truncated,
        };
        self.sessions.insert(self.next, session);
        self.next += 1;
        Ok(info)
    }

    fn open_xlsx(&mut self, path: &Path) -> Result<ViewerSession, IpcError> {
        let workbook: Xlsx<BufReader<File>> =
            calamine::open_workbook(path).map_err(|e| IpcError::new("io", format!("打开 xlsx 失败：{e}")))?;
        let sheets = workbook.sheet_names().to_vec();
        Ok(ViewerSession {
            kind: ViewerKind::Xlsx,
            workbook: Some(workbook),
            sheets,
            docx_lines: None,
            docx_truncated: false,
        })
    }

    fn open_docx(&mut self, path: &Path) -> Result<ViewerSession, IpcError> {
        let lines = extract_docx_paragraphs(path)?;
        Ok(ViewerSession {
            kind: ViewerKind::Docx,
            workbook: None,
            sheets: vec!["正文".into()],
            docx_lines: Some(lines.0),
            docx_truncated: lines.1,
        })
    }

    /// PDF text extraction (proposal increment 1): no layout, same single-
    /// column path as docx. `pdf-extract` is known to panic on some malformed
    /// files — the call is unwound into an error, never into the IPC thread.
    fn open_pdf(&mut self, path: &Path) -> Result<ViewerSession, IpcError> {
        let shown = path.display().to_string();
        let extracted = match std::panic::catch_unwind(|| pdf_extract::extract_text(&shown)) {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => {
                return Err(IpcError::new(
                    "invalid_argument",
                    format!("PDF 文本提取失败：{e}"),
                ))
            }
            Err(_) => {
                return Err(IpcError::new(
                    "invalid_argument",
                    "PDF 解析器内部异常".to_string(),
                ))
            }
        };
        let mut lines: Vec<String> = Vec::new();
        let mut total = 0usize;
        let mut truncated = false;
        for line in extracted.lines() {
            if lines.len() >= MAX_PARAGRAPHS || total + line.len() > MAX_PARA_TEXT_BYTES {
                truncated = true;
                break;
            }
            total += line.len();
            lines.push(line.to_string());
        }
        if lines.is_empty() && !truncated {
            // A scanned/empty PDF: still a valid session with zero rows.
            lines.push(String::new());
        }
        Ok(ViewerSession {
            kind: ViewerKind::Pdf,
            workbook: None,
            sheets: vec!["正文".into()],
            docx_lines: Some(lines),
            docx_truncated: truncated,
        })
    }

    pub(crate) fn close(&mut self, id: u64) -> bool {
        self.sessions.remove(&id).is_some()
    }

    pub(crate) fn rows(
        &mut self,
        id: u64,
        sheet_index: usize,
        from_row: u64,
        limit: u32,
    ) -> Result<ViewerPageJson, IpcError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| IpcError::new("unknown_request", format!("no viewer session {id}")))?;
        if limit == 0 || limit > MAX_WINDOW {
            return Err(IpcError::new(
                "invalid_argument",
                format!("limit 须为 1–{MAX_WINDOW}"),
            ));
        }
        // Text-line kinds (docx / pdf) share the single-column path; the
        // workbook branch is the only other shape.
        if let Some(lines) = &session.docx_lines {
            let total = lines.len() as u64;
            let start = from_row.min(total) as usize;
            let end = (start + limit as usize).min(lines.len());
            let rows = lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, text)| ViewerRowJson {
                    line: start as u64 + i as u64 + 1,
                    cells: vec![text.clone()],
                })
                .collect();
            Ok(ViewerPageJson {
                total,
                columns: vec!["内容".into()],
                rows,
            })
        } else {
            let wb = session.workbook.as_mut().expect("xlsx workbook");
                let name = session
                    .sheets
                    .get(sheet_index)
                    .ok_or_else(|| IpcError::new("invalid_argument", "sheet 越界"))?
                    .clone();
                let range = wb
                    .worksheet_range(&name)
                    .map_err(|e| IpcError::new("io", format!("读取工作表失败：{e}")))?;
                let total = range.height() as u64;
                let start = from_row.min(total) as usize;
                let end = (start + limit as usize).min(range.height());
                let mut raw: Vec<Vec<String>> = Vec::with_capacity(end - start);
                for row in range.rows().skip(start).take(end - start) {
                    raw.push(row.iter().map(cell_to_string).collect());
                }
                // Window width = the furthest non-empty cell in the window
                // (trailing empty columns add nothing), capped at MAX_COLUMNS.
                let mut width = 1usize;
                for r in &raw {
                    let last = r
                        .iter()
                        .rposition(|c| !c.is_empty())
                        .map(|p| p + 1)
                        .unwrap_or(1);
                    width = width.max(last.min(MAX_COLUMNS));
                }
                let rows = raw
                    .into_iter()
                    .enumerate()
                    .map(|(i, mut cells)| {
                        cells.truncate(width);
                        cells.resize(width, String::new());
                        ViewerRowJson {
                            line: start as u64 + i as u64 + 1,
                            cells,
                        }
                    })
                    .collect();
                Ok(ViewerPageJson {
                    total,
                    columns: (0..width).map(column_letter).collect(),
                    rows,
                })
        }
    }

    /// CSV (xlsx, raw dump without a header row — spreadsheet data *is* the
    /// table) / TXT (docx paragraphs, pdf extracted lines). Same cell mapping
    /// as `rows`.
    pub(crate) fn export(
        &mut self,
        id: u64,
        sheet_index: usize,
        target: &Path,
    ) -> Result<ViewerExportStats, IpcError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| IpcError::new("unknown_request", format!("no viewer session {id}")))?;
        let tmp = target.with_file_name(format!(
            ".{}.tmp-{}-{}",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("viewer-out"),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let result = if session.docx_lines.is_some() {
            export_lines_txt(session, &tmp)
        } else {
            export_xlsx_csv(session, sheet_index, &tmp)
        };
        match result {
            Ok(written) => {
                std::fs::rename(&tmp, target)
                    .map_err(|e| IpcError::new("io", format!("保存导出失败：{e}")))?;
                Ok(ViewerExportStats { written })
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

fn export_lines_txt(session: &ViewerSession, tmp: &Path) -> Result<u64, IpcError> {
    let lines = session.docx_lines.as_ref().expect("text lines");
    let mut out = std::io::BufWriter::new(
        File::create(tmp).map_err(|e| IpcError::new("io", format!("创建导出文件失败：{e}")))?,
    );
    for (i, text) in lines.iter().enumerate() {
        if i > 0 {
            out.write_all(b"\n")
                .map_err(|e| IpcError::new("io", format!("写入导出失败：{e}")))?;
        }
        out.write_all(text.as_bytes())
            .map_err(|e| IpcError::new("io", format!("写入导出失败：{e}")))?;
    }
    Ok(lines.len() as u64)
}

fn export_xlsx_csv(
    session: &mut ViewerSession,
    sheet_index: usize,
    tmp: &Path,
) -> Result<u64, IpcError> {
    let wb = session.workbook.as_mut().expect("xlsx workbook");
    let name = session
        .sheets
        .get(sheet_index)
        .ok_or_else(|| IpcError::new("invalid_argument", "sheet 越界"))?
        .clone();
    let range = wb
        .worksheet_range(&name)
        .map_err(|e| IpcError::new("io", format!("读取工作表失败：{e}")))?;
    let mut out = std::io::BufWriter::new(
        File::create(tmp).map_err(|e| IpcError::new("io", format!("创建导出文件失败：{e}")))?,
    );
    // UTF-8 BOM so Excel opens the file as UTF-8 without an import wizard.
    out.write_all(&[0xEF, 0xBB, 0xBF])
        .map_err(|e| IpcError::new("io", format!("写入导出失败：{e}")))?;
    let mut written = 0u64;
    for row in range.rows() {
        if written >= MAX_EXPORT_ROWS {
            break;
        }
        let cells: Vec<String> = row.iter().map(cell_to_string).collect();
        let line = cells
            .iter()
            .map(|c| {
                if c.contains(',') || c.contains('"') || c.contains('\n') {
                    format!("\"{}\"", c.replace('"', "\"\""))
                } else {
                    c.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        out.write_all(line.as_bytes())
            .and_then(|_| out.write_all(b"\n"))
            .map_err(|e| IpcError::new("io", format!("写入导出失败：{e}")))?;
        written += 1;
    }
    out.flush().map_err(|e| IpcError::new("io", format!("写入导出失败：{e}")))?;
    Ok(written)
}

/// Spreadsheet-column letters: 0→A, 25→Z, 26→AA (capped callers ensure < 64).
fn column_letter(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push((b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    s.into_iter().collect()
}

fn cell_to_string(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::DateTime(dt) => dt
            .as_datetime()
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| format!("{}", dt.as_f64())),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e}"),
    }
}

/// Extract `word/document.xml` paragraphs. Text inside one paragraph may be
/// split across several `w:t` runs — concatenated verbatim; an empty `<w:p/>`
/// still counts as one (empty) line.
fn extract_docx_paragraphs(path: &Path) -> Result<(Vec<String>, bool), IpcError> {
    let file = File::open(path).map_err(|e| IpcError::new("io", format!("打开 docx 失败：{e}")))?;
    let mut archive =
        zip::ZipArchive::new(BufReader::new(file)).map_err(|e| IpcError::new("io", format!("打开 docx 失败：{e}")))?;
    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|e| IpcError::new("io", format!("docx 缺少 word/document.xml：{e}")))?;
    if entry.size() > MAX_XML_ENTRY_BYTES {
        return Err(IpcError::new(
            "invalid_argument",
            format!("document.xml 超过 {MAX_XML_ENTRY_BYTES} 字节上限"),
        ));
    }
    let mut xml = String::new();
    entry
        .read_to_string(&mut xml)
        .map_err(|e| IpcError::new("io", format!("读取 document.xml 失败：{e}")))?;

    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut paras: Vec<String> = Vec::new();
    let mut total_bytes = 0usize;
    let mut truncated = false;
    let mut cur = String::new();
    let mut in_t = false;
    let push = |paras: &mut Vec<String>, cur: &mut String, total: &mut usize, truncated: &mut bool| {
        if paras.len() >= MAX_PARAGRAPHS || *total + cur.len() > MAX_PARA_TEXT_BYTES {
            *truncated = true;
            cur.clear();
            return;
        }
        *total += cur.len();
        paras.push(std::mem::take(cur));
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"w:p" => cur.clear(),
                b"w:t" => in_t = true,
                _ => {}
            },
            Ok(Event::End(e)) => {
                let qname = e.name();
                let name = qname.as_ref();
                if name == b"w:t" {
                    in_t = false;
                } else if name == b"w:p" {
                    push(&mut paras, &mut cur, &mut total_bytes, &mut truncated);
                    if truncated {
                        break;
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                // `<w:p/>` self-closes: an empty paragraph still is a line.
                if e.name().as_ref() == b"w:p" {
                    push(&mut paras, &mut cur, &mut total_bytes, &mut truncated);
                    if truncated {
                        break;
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if in_t {
                    match t.unescape() {
                        Ok(s) => cur.push_str(&s),
                        Err(_) => cur.push_str(&String::from_utf8_lossy(t.as_ref())),
                    }
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(IpcError::new(
                    "invalid_argument",
                    format!("document.xml 解析失败：{e}"),
                ))
            }
        }
    }
    if !cur.is_empty() && !truncated {
        // A document.xml ending without closing the last paragraph is malformed,
        // but the text still belongs to the user.
        push(&mut paras, &mut cur, &mut total_bytes, &mut truncated);
    }
    Ok((paras, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumelog-viewer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_xlsx(path: &Path) {
        use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        ws.set_name("数据").unwrap();
        ws.write(0, 0, "甲").unwrap();
        ws.write(0, 1, 42).unwrap();
        ws.write(1, 0, 3.5).unwrap();
        ws.write(1, 1, true).unwrap();
        // A date cell only reads back as DateTime when the num-format is a
        // date pattern — plain write_datetime keeps the serial visible.
        let dt_fmt = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
        let dt = ExcelDateTime::from_ymd(2026, 10, 30).unwrap().and_hms(12, 30, 0).unwrap();
        ws.write_datetime_with_format(1, 2, dt, &dt_fmt).unwrap();
        // row 3: only column A populated (trailing-empty trim case)
        ws.write(2, 0, "只有A列").unwrap();
        let ws2 = wb.add_worksheet();
        ws2.set_name("Sheet2").unwrap();
        ws2.write(0, 0, "second").unwrap();
        wb.save(path).unwrap();
    }

    fn write_docx(path: &Path) {
        let file = File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        zw.start_file("word/document.xml", opts).unwrap();
        zw.write_all(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>World</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">第二段  保留空格</w:t></w:r></w:p><w:p/></w:body></w:document>"#.as_bytes(),
        )
        .unwrap();
        zw.finish().unwrap();
    }

    /// Hand-assembled minimal single-page PDF: one Helvetica text object per
    /// entry, newlines between them. Good enough for `pdf-extract`.
    fn write_pdf(path: &Path, lines: &[&str]) {
        let mut content = String::from("BT\n/F1 12 Tf\n72 720 Td\n");
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                content.push_str("0 -18 Td\n");
            }
            content.push_str(&format!("({line}) Tj\n"));
        }
        content.push_str("ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut body = Vec::from(&b"%PDF-1.4\n"[..]);
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(body.len());
            body.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref_pos = body.len();
        body.extend_from_slice(format!("xref\n0 {}\n", objs.len() + 1).as_bytes());
        body.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets {
            body.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        body.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn xlsx_open_rows_and_formatting() {
        let dir = temp_dir("xlsx");
        let p = dir.join("book.xlsx");
        write_xlsx(&p);
        let mut sessions = ViewerSessions::default();
        let info = sessions.open(&p).unwrap();
        assert_eq!(info.kind, "xlsx");
        assert_eq!(info.sheets, vec!["数据", "Sheet2"]);

        let page = sessions.rows(info.id, 0, 0, 100).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.columns.len(), 3, "{:?}", page.columns);
        assert_eq!(page.rows[0].cells[0], "甲");
        assert_eq!(page.rows[0].cells[1], "42"); // integer float de-tailed
        assert_eq!(page.rows[1].cells[0], "3.5");
        assert_eq!(page.rows[1].cells[1], "TRUE");
        assert_eq!(page.rows[1].cells[2], "2026-10-30 12:30:00");
        // uniform window width: row 3 pads up to the widest row in the window
        assert_eq!(page.rows[2].cells, vec!["只有A列", "", ""]);

        // windowing
        let page2 = sessions.rows(info.id, 0, 2, 1000).unwrap();
        assert_eq!(page2.rows.len(), 1);
        assert_eq!(page2.rows[0].line, 3);

        // second sheet
        let page3 = sessions.rows(info.id, 1, 0, 100).unwrap();
        assert_eq!(page3.rows[0].cells[0], "second");

        // bad sheet index / unknown session
        assert!(sessions.rows(info.id, 9, 0, 10).is_err());
        assert!(sessions.rows(999, 0, 0, 10).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn docx_paragraphs_across_runs() {
        let dir = temp_dir("docx");
        let p = dir.join("doc.docx");
        write_docx(&p);
        let mut sessions = ViewerSessions::default();
        let info = sessions.open(&p).unwrap();
        assert_eq!(info.kind, "docx");
        assert_eq!(info.paragraphs, Some(3));
        assert!(!info.truncated);

        let page = sessions.rows(info.id, 0, 0, 100).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.columns, vec!["内容"]);
        assert_eq!(page.rows[0].cells, vec!["Hello World"]);
        assert_eq!(page.rows[1].cells, vec!["第二段  保留空格"]);
        assert_eq!(page.rows[2].cells, vec![""]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pdf_text_extraction_and_malformed_input() {
        let dir = temp_dir("pdf");
        let p = dir.join("doc.pdf");
        write_pdf(&p, &["Hello PDF", "Second line"]);
        let mut sessions = ViewerSessions::default();
        let info = sessions.open(&p).unwrap();
        assert_eq!(info.kind, "pdf");
        assert!(!info.truncated);

        let page = sessions.rows(info.id, 0, 0, 1000).unwrap();
        let joined = page
            .rows
            .iter()
            .map(|r| r.cells.join(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Hello PDF"), "{joined:?}");
        assert!(joined.contains("Second line"), "{joined:?}");
        assert_eq!(page.columns, vec!["内容"]);

        // TXT export rides the same text-lines path as docx.
        let t = dir.join("out.txt");
        let stats = sessions.export(info.id, 0, &t).unwrap();
        let text = std::fs::read_to_string(&t).unwrap();
        assert!(text.contains("Hello PDF") && text.contains("Second line"), "{text:?}");
        assert!(stats.written >= 1);

        // Malformed input: an error surfaced to the user, never a panic.
        let bad = dir.join("bad.pdf");
        std::fs::write(&bad, b"this is not a pdf at all").unwrap();
        let err = sessions.open(&bad).unwrap_err();
        assert!(
            err.code == "invalid_argument" || err.code == "io",
            "unexpected code {} ({})",
            err.code,
            err.detail.as_deref().unwrap_or("")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsupported_extension_and_session_cap() {
        let dir = temp_dir("misc");
        let txt = dir.join("a.txt");
        std::fs::write(&txt, "x").unwrap();
        let mut sessions = ViewerSessions::default();
        let err = sessions.open(&txt).unwrap_err();
        assert_eq!(err.code, "viewer_unsupported");

        for i in 0..4 {
            let p = dir.join(format!("b{i}.docx"));
            write_docx(&p);
            sessions.open(&p).unwrap();
        }
        let p5 = dir.join("c.docx");
        write_docx(&p5);
        let err = sessions.open(&p5).unwrap_err();
        assert_eq!(err.code, "invalid_argument");

        // close is idempotent and frees a slot
        let first = sessions.sessions.keys().copied().next().unwrap();
        assert!(sessions.close(first));
        assert!(!sessions.close(first));
        assert!(sessions.open(&p5).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exports_match_table_and_escape() {
        let dir = temp_dir("exp");
        let xp = dir.join("book.xlsx");
        write_xlsx(&xp);
        let dp = dir.join("doc.docx");
        write_docx(&dp);
        let mut sessions = ViewerSessions::default();
        let xi = sessions.open(&xp).unwrap();
        let di = sessions.open(&dp).unwrap();

        let target = dir.join("out.csv");
        let stats = sessions.export(xi.id, 0, &target).unwrap();
        assert_eq!(stats.written, 3);
        let raw = std::fs::read(&target).unwrap();
        assert!(raw.starts_with(&[0xEF, 0xBB, 0xBF]), "BOM required");
        let text = String::from_utf8_lossy(&raw[3..]).into_owned();
        assert!(text.lines().count() == 3, "{text}");
        assert!(text.contains("甲,42"));
        assert!(text.contains("2026-10-30 12:30:00"));

        let t2 = dir.join("out.txt");
        let stats = sessions.export(di.id, 0, &t2).unwrap();
        assert_eq!(stats.written, 3);
        let text = std::fs::read_to_string(&t2).unwrap();
        assert_eq!(text, "Hello World\n第二段  保留空格\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn column_letters() {
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(25), "Z");
        assert_eq!(column_letter(26), "AA");
        assert_eq!(column_letter(63), "BL");
    }
}
