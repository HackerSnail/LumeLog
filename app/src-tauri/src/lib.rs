//! LumeLog Tauri v2 entry point. The whole point of this crate is to be THIN:
//! every command forwards to `lumelog-core` (spec §3 — heavy logic lives in the
//! pure-Rust crate, reused by the CLI in Task 7 and this shell). Streams that
//! the front end consumes are delivered over `tauri::ipc::Channel`s.

mod commands;
mod dto;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppDocs::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_ping,
            commands::doc_open,
            commands::doc_chunk,
            commands::doc_save,
            commands::doc_list,
            commands::search_run,
            commands::search_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LumeLog");
}
