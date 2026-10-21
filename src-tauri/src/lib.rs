pub mod askpass;
mod commands;
mod credentials;
mod editor;
pub mod hosts;
mod prefs;
mod process;
mod resolve;
mod sftp;
mod ssh_bin;
mod terminal;
mod transfer;

use tauri::{Emitter, Manager};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(terminal::Terminals::default())
        .manage(transfer::Transfers::default())
        .setup(|app| {
            let handle = app.handle().clone();
            let broker = askpass::Broker::new(
                std::env::current_exe()?,
                credentials::CredentialManager,
                move |question| handle.emit("askpass", question).map_err(|e| e.to_string()),
            );
            tauri::async_runtime::block_on(broker.start())?;
            app.manage(broker);
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                disable_browser_accelerator_keys(&window)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_hosts,
            commands::host_details,
            commands::open_config,
            commands::prefs_load,
            commands::prefs_set_config_path,
            commands::prefs_set_layout,
            commands::prefs_record_host,
            commands::sftp_list,
            commands::sftp_upload,
            commands::sftp_download,
            commands::transfer_cancel,
            commands::askpass_reply,
            commands::askpass_end,
            commands::terminal_open,
            commands::terminal_write,
            commands::terminal_resize,
            commands::terminal_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}

/// WebView2 otherwise handles F3, F5, F7, F12, Ctrl+F, Ctrl+P, etc. itself,
/// before the page sees them, which would steal Byobu's function keys.
/// Tauri does not expose this setting, so it is set on the WebView2 directly.
#[cfg(windows)]
fn disable_browser_accelerator_keys(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows::core::Interface;

    window.with_webview(|webview| {
        // SAFETY: COM calls on the live controller, on the thread that owns it.
        let result = unsafe {
            webview
                .controller()
                .CoreWebView2()
                .and_then(|core| core.Settings())
                .and_then(|settings| settings.cast::<ICoreWebView2Settings3>())
                .and_then(|settings| settings.SetAreBrowserAcceleratorKeysEnabled(false))
        };
        if let Err(e) = result {
            eprintln!("could not disable WebView2 accelerator keys: {e}");
        }
    })
}
