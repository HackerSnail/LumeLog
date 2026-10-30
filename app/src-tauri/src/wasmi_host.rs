//! wasmi host for `plugins/*.wasm` parser plugins (plugin interface, increment
//! 3 — spec `docs/specs/2026-10-30-plugin-wasm-impl-design.md`).
//!
//! ABI (plain wasm, no wit): the module exports `memory`, `alloc(len) -> ptr`,
//! `meta() -> i64`, `parse(input_ptr, input_len, base_line) -> i64` and
//! `finish() -> i64`; every `i64` result packs `(ptr << 32) | len` pointing at
//! UTF-8 JSON in plugin memory. Host flow per chunk: `alloc` → write bytes →
//! reset fuel → `parse` → read JSON rows. Interpreter + per-call fuel bound
//! runaway plugins; the run loop checks cancel/deadline between chunks.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wasmi::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc, Memory};

use crate::commands::DocLines;
use crate::plugins::{ParserPluginInfo, ParserRowJson, PARSER_BATCH, RUN_SECONDS};

/// Source wasm modules are small; refuse anything bigger.
pub(crate) const MAX_WASM_BYTES: u64 = 8 * 1024 * 1024;
/// Plugin linear memory cap (wasmi StoreLimits).
const MAX_WASM_MEMORY: usize = 16 * 1024 * 1024;
/// Per-call fuel: bounds a single `parse` (an infinite loop in the plugin
/// cannot outlive this many instructions).
const FUEL_PER_CALL: u64 = 500_000_000;
/// Accumulate document lines up to this size before calling `parse`.
const CHUNK_BYTES: usize = 256 * 1024;
/// One packed result may not exceed this (JSON rows for a 256KB chunk).
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct WasmPlugin {
    store: Store<StoreLimits>,
    memory: Memory,
    meta_fn: TypedFunc<(), i64>,
    alloc_fn: TypedFunc<i32, i32>,
    parse_fn: TypedFunc<(i32, i32, i64), i64>,
    finish_fn: TypedFunc<(), i64>,
}

fn unpack(ret: i64) -> (usize, usize) {
    let bits = ret as u64;
    ((bits >> 32) as usize, (bits & 0xFFFF_FFFF) as usize)
}

impl WasmPlugin {
    pub(crate) fn instantiate(wasm: &[u8]) -> Result<Self, String> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module =
            Module::new(&engine, wasm).map_err(|e| format!("wasm 模块无效：{e}"))?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_WASM_MEMORY)
            .build();
        let mut store = Store::new(&engine, limits);
        store.limiter(|limits| limits);
        store
            .set_fuel(FUEL_PER_CALL)
            .map_err(|e| format!("设置 fuel 失败：{e}"))?;
        let linker = <Linker<StoreLimits>>::new(&engine);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("wasm 实例化失败：{e}"))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or("插件必须导出 memory")?;
        let missing = |name: &str| format!("插件缺少导出函数 `{name}`");
        Ok(Self {
            meta_fn: instance
                .get_typed_func::<(), i64>(&store, "meta")
                .map_err(|_| missing("meta"))?,
            alloc_fn: instance
                .get_typed_func::<i32, i32>(&store, "alloc")
                .map_err(|_| missing("alloc"))?,
            parse_fn: instance
                .get_typed_func::<(i32, i32, i64), i64>(&store, "parse")
                .map_err(|_| missing("parse"))?,
            finish_fn: instance
                .get_typed_func::<(), i64>(&store, "finish")
                .map_err(|_| missing("finish"))?,
            memory,
            store,
        })
    }

    /// Read a `(ptr << 32) | len` result out of plugin memory.
    fn read_packed(&mut self, ret: i64) -> Result<Vec<u8>, String> {
        let (ptr, len) = unpack(ret);
        if len == 0 {
            return Ok(Vec::new());
        }
        if len > MAX_OUTPUT_BYTES {
            return Err(format!("单次调用输出超过 {MAX_OUTPUT_BYTES} 字节"));
        }
        let mut buf = vec![0u8; len];
        self.memory
            .read(&self.store, ptr, &mut buf)
            .map_err(|e| format!("读回插件输出失败：{e}"))?;
        Ok(buf)
    }

    /// Call `meta()` and parse the plugin-info JSON.
    pub(crate) fn meta(&mut self) -> Result<ParserPluginInfo, String> {
        self.store
            .set_fuel(FUEL_PER_CALL)
            .map_err(|e| format!("设置 fuel 失败：{e}"))?;
        let ret = self
            .meta_fn
            .call(&mut self.store, ())
            .map_err(|e| format!("meta 调用失败：{e}"))?;
        let out = self.read_packed(ret)?;
        serde_json::from_slice(&out).map_err(|e| format!("meta JSON 无效：{e}"))
    }

    fn call_rows(
        &mut self,
        name: &str,
        chunk: Option<&[u8]>,
        base_line: u64,
    ) -> Result<Vec<ParserRowJson>, String> {
        let rows = match chunk {
            Some(bytes) => {
                let ptr = self
                    .alloc_fn
                    .call(&mut self.store, bytes.len() as i32)
                    .map_err(|e| format!("alloc 调用失败：{e}"))?;
                self.memory
                    .write(&mut self.store, ptr as usize, bytes)
                    .map_err(|e| format!("写入插件内存失败：{e}"))?;
                self.store
                    .set_fuel(FUEL_PER_CALL)
                    .map_err(|e| format!("设置 fuel 失败：{e}"))?;
                let ret = self
                    .parse_fn
                    .call(&mut self.store, (ptr, bytes.len() as i32, base_line as i64))
                    .map_err(|e| format!("{name} 调用失败：{e}"))?;
                self.read_packed(ret)?
            }
            None => {
                self.store
                    .set_fuel(FUEL_PER_CALL)
                    .map_err(|e| format!("设置 fuel 失败：{e}"))?;
                let ret = self
                    .finish_fn
                    .call(&mut self.store, ())
                    .map_err(|e| format!("{name} 调用失败：{e}"))?;
                self.read_packed(ret)?
            }
        };
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_slice(&rows).map_err(|e| format!("{name} 输出 JSON 无效：{e}"))
    }

    /// Feed one whole-lines chunk; `base_line` is the 1-based number of the
    /// chunk's first line.
    pub(crate) fn parse_chunk(
        &mut self,
        chunk: &[u8],
        base_line: u64,
    ) -> Result<Vec<ParserRowJson>, String> {
        self.call_rows("parse", Some(chunk), base_line)
    }

    /// Flush the plugin's internal tail buffer at end of stream.
    pub(crate) fn finish_rows(&mut self) -> Result<Vec<ParserRowJson>, String> {
        self.call_rows("finish", None, 0)
    }
}

/// Shared state for one `run_wasm_parser` invocation; `flush` funnels both the
/// loop path and the EOF tail path so cancel/deadline/error handling exists
/// exactly once.
struct RunCtx<'a> {
    stats: &'a mut crate::plugins::ParserStats,
    batch: &'a mut Vec<ParserRowJson>,
    cancel: &'a Arc<dyn Fn() -> bool + Send + Sync>,
    start: Instant,
    cancelled: bool,
}

impl<'a> RunCtx<'a> {
    /// Call `parse` (chunk) / `finish` (None); returns false when the run must
    /// stop (cancel, deadline, or plugin error — already recorded in stats).
    fn flush(
        &mut self,
        plugin: &mut WasmPlugin,
        chunk: Option<&[u8]>,
        base: u64,
        script_error: &mut Option<String>,
    ) -> bool {
        if (self.cancel)() {
            self.cancelled = true;
            return false;
        }
        if self.start.elapsed() > Duration::from_secs(RUN_SECONDS) {
            *script_error = Some(format!("解析超时（{RUN_SECONDS}s）"));
            return false;
        }
        let result = match chunk {
            Some(bytes) => plugin.parse_chunk(bytes, base),
            None => plugin.finish_rows(),
        };
        match result {
            Ok(mut rows) => {
                self.stats.rows += rows.len() as u64;
                self.batch.append(&mut rows);
                true
            }
            Err(e) => {
                *script_error = Some(e);
                false
            }
        }
    }
}

/// Stream the document through a wasm parser: accumulate whole lines into
/// 256KB chunks, `parse` each, `finish` at EOF. Same batching/limits contract
/// as `run_parser_plugin` (script engine).
pub(crate) fn run_wasm_parser(
    doc: Arc<Mutex<lumelog_core::doc::DocFile>>,
    plugin_path: &Path,
    on_batch: &mut dyn FnMut(Vec<ParserRowJson>),
    cancel: Arc<dyn Fn() -> bool + Send + Sync>,
) -> crate::plugins::ParserStats {
    let mut stats = crate::plugins::ParserStats::default();
    if let Ok(meta) = std::fs::metadata(plugin_path) {
        if meta.len() > MAX_WASM_BYTES {
            stats.error = Some(format!("wasm 文件过大（超过 {MAX_WASM_BYTES} 字节）"));
            return stats;
        }
    }
    let wasm = match std::fs::read(plugin_path) {
        Ok(b) => b,
        Err(e) => {
            stats.error = Some(format!("读取插件失败：{e}"));
            return stats;
        }
    };
    let mut plugin = match WasmPlugin::instantiate(&wasm) {
        Ok(p) => p,
        Err(e) => {
            stats.error = Some(e);
            return stats;
        }
    };
    let mut lines = DocLines::new(doc);
    let mut ctx = RunCtx {
        stats: &mut stats,
        batch: &mut Vec::new(),
        cancel: &cancel,
        start: Instant::now(),
        cancelled: false,
    };
    let mut script_error: Option<String> = None;
    let mut buf: Vec<u8> = Vec::new();
    let mut base_line: u64 = 1;
    for (line_number, text) in lines.by_ref() {
        if (ctx.cancel)() {
            ctx.cancelled = true;
            break;
        }
        if ctx.start.elapsed() > Duration::from_secs(RUN_SECONDS) {
            script_error = Some(format!("解析超时（{RUN_SECONDS}s）"));
            break;
        }
        ctx.stats.lines += 1;
        buf.extend_from_slice(text.as_bytes());
        buf.push(b'\n');
        if buf.len() < CHUNK_BYTES {
            continue;
        }
        let ok = ctx.flush(&mut plugin, Some(&buf), base_line, &mut script_error);
        base_line = line_number + 1;
        buf.clear();
        if !ok {
            break;
        }
        if ctx.batch.len() >= PARSER_BATCH {
            on_batch(std::mem::take(&mut *ctx.batch));
        }
    }
    // EOF: flush the tail chunk, then let the plugin flush its own buffer.
    if script_error.is_none() && !ctx.cancelled && !buf.is_empty() {
        ctx.flush(&mut plugin, Some(&buf), base_line, &mut script_error);
    }
    if script_error.is_none() && !ctx.cancelled {
        match plugin.finish_rows() {
            Ok(mut rows) => {
                ctx.stats.rows += rows.len() as u64;
                ctx.batch.append(&mut rows);
            }
            Err(e) => script_error = Some(e),
        }
    }
    if !ctx.batch.is_empty() {
        on_batch(std::mem::take(&mut *ctx.batch));
    }
    stats.cancelled = ctx.cancelled;
    stats.error = script_error;
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{find_parser_plugin, list_parser_plugins};
    use wasm_encoder::{
        CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function, FunctionSection,
        Instruction, MemorySection, MemoryType, Module, TypeSection, ValType,
    };

    const META_JSON: &str =
        r#"{"id":"wfmt","name":"W 插件","extensions":["wf"],"columns":[{"key":"k","label":"K"}]}"#;
    const ROW_JSON: &str = r#"[{"line":1,"cells":["x"]}]"#;

    /// Build a minimal parser module. `row_ptr/row_len` point at a constant
    /// row-JSON blob for `parse`/`finish` (0 → empty output).
    fn build_module(row_ptr: u64, row_len: u64) -> Vec<u8> {
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I64]);
        types.ty().function([ValType::I32], [ValType::I32]);
        types.ty().function(
            [ValType::I32, ValType::I32, ValType::I64],
            [ValType::I64],
        );
        module.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        funcs.function(1);
        funcs.function(2);
        funcs.function(0);
        module.section(&funcs);
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memories);
        let mut exports = ExportSection::new();
        exports.export("memory", ExportKind::Memory, 0);
        exports.export("meta", ExportKind::Func, 0);
        exports.export("alloc", ExportKind::Func, 1);
        exports.export("parse", ExportKind::Func, 2);
        exports.export("finish", ExportKind::Func, 3);
        module.section(&exports);
        let packed_meta = ((2048u64) << 32) | META_JSON.len() as u64;
        let packed_row = (row_ptr << 32) | row_len;
        let mut code = CodeSection::new();
        let mut meta_fn = Function::new([]);
        meta_fn.instruction(&Instruction::I64Const(packed_meta as i64));
        meta_fn.instruction(&Instruction::End);
        code.function(&meta_fn);
        let mut alloc_fn = Function::new([]);
        alloc_fn.instruction(&Instruction::LocalGet(0));
        alloc_fn.instruction(&Instruction::End);
        code.function(&alloc_fn);
        let mut parse_fn = Function::new([]);
        parse_fn.instruction(&Instruction::I64Const(packed_row as i64));
        parse_fn.instruction(&Instruction::End);
        code.function(&parse_fn);
        let mut finish_fn = Function::new([]);
        finish_fn.instruction(&Instruction::I64Const(packed_row as i64));
        finish_fn.instruction(&Instruction::End);
        code.function(&finish_fn);
        module.section(&code);
        let mut data = DataSection::new();
        let meta_blob: Vec<u8> = META_JSON.bytes().collect();
        data.active(0, &ConstExpr::raw([0x41, 0x80, 0x10]), meta_blob); // i32.const 2048 (LEB)
        if row_len > 0 {
            let blob: Vec<u8> = ROW_JSON.bytes().collect();
            data.active(0, &ConstExpr::raw([0x41, 0x80, 0x20]), blob); // i32.const 4096
        }
        module.section(&data);
        module.finish()
    }

    fn open_doc(dir: &std::path::Path, body: &str) -> Arc<Mutex<lumelog_core::doc::DocFile>> {
        let doc_path = dir.join("in.log");
        std::fs::write(&doc_path, body).unwrap();
        Arc::new(Mutex::new(
            lumelog_core::doc::DocFile::open(&doc_path, 1 << 20).unwrap(),
        ))
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumelog-wasm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wasm_metadata_and_constant_rows() {
        let dir = temp_dir("meta");
        let plugin_path = dir.join("w.wasm");
        std::fs::write(&plugin_path, build_module(4096, ROW_JSON.len() as u64)).unwrap();

        let (parsers, errors) = list_parser_plugins(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(parsers.len(), 1);
        assert_eq!(parsers[0].id, "wfmt");
        assert_eq!(parsers[0].columns[0].key, "k");

        let found = find_parser_plugin(&dir, "wfmt").unwrap();
        assert_eq!(found, plugin_path);

        let stats = run_wasm_parser(
            open_doc(&dir, "hello\n"),
            &plugin_path,
            &mut |rows| assert!(!rows.is_empty()),
            Arc::new(|| false),
        );
        assert_eq!(stats.error, None, "{stats:?}");
        assert_eq!(stats.lines, 1);
        // tail chunk (1 row) + finish flush (1 row)
        assert_eq!(stats.rows, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wasm_empty_plugin_and_broken_module() {
        let dir = temp_dir("wempty");
        let good = dir.join("a-good.wasm");
        std::fs::write(&good, build_module(0, 0)).unwrap();
        let bad = dir.join("b-bad.wasm");
        std::fs::write(&bad, b"\0asm garbage").unwrap();

        let (parsers, errors) = list_parser_plugins(&dir);
        assert_eq!(parsers.len(), 1, "{errors:?}");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("b-bad.wasm:"), "{errors:?}");

        let stats = run_wasm_parser(
            open_doc(&dir, "a\nb\n"),
            &good,
            &mut |_| {},
            Arc::new(|| false),
        );
        assert_eq!(stats.error, None, "{stats:?}");
        assert_eq!(stats.lines, 2);
        assert_eq!(stats.rows, 0); // parse/finish return empty output
        std::fs::remove_dir_all(&dir).ok();
    }
}
