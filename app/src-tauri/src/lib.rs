//! LumeLog Tauri v2 entry point. The whole point of this crate is to be THIN:
//! every command forwards to `lumelog-core` (spec §3 — heavy logic lives in the
//! pure-Rust crate, reused by the CLI in Task 7 and this shell). Streams that
//! the front end consumes are delivered over `tauri::ipc::Channel`s.

mod can_export;
mod commands;
mod dto;
mod keymap;
mod mf4;
mod plugins;
mod recovery;
mod script;
mod state;
mod watch;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppDocs::default())
        .manage(mf4::Mf4Sessions::default())
        .manage(watch::AppWatch::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_ping,
            commands::doc_open,
            commands::doc_chunk,
            commands::doc_reload,
            commands::doc_tail_append,
            commands::doc_set_encoding,
            commands::doc_close,
            commands::watch_subscribe,
            commands::watch_status,
            commands::keymap_get,
            commands::keymap_set,
            commands::keymap_reset,
            commands::mf4_open,
            commands::mf4_close,
            commands::mf4_channels,
            commands::mf4_records,
            commands::mf4_series,
            commands::mf4_summary,
            commands::mf4_export_csv,
            commands::mf4_export_cancel,
            commands::script_list,
            commands::script_read,
            commands::script_write,
            commands::script_delete,
            commands::script_run,
            commands::script_cancel,
            plugins::syntax_plugins_list,
            plugins::parser_plugins_list,
            commands::doc_save,
            commands::doc_list,
            commands::search_run,
            commands::search_cancel,
            commands::dbc_import,
            commands::can_parse,
            commands::can_parse_blf,
            commands::can_cancel,
            commands::can_decode_snippet,
            commands::filter_run,
            commands::filter_cancel,
            commands::doc_lines_at,
            commands::export_lines,
            commands::export_cancel,
            commands::export_csv,
            commands::export_jsonl,
            commands::can_export_blf_csv,
            commands::can_export_blf_jsonl,
            commands::export_can_cancel,
            commands::recovery_save,
            commands::recovery_list,
            commands::recovery_load,
            commands::recovery_discard,
            commands::recovery_enabled,
            commands::recovery_set_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LumeLog");
}
