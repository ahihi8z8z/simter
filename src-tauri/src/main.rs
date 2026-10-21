// Release builds are GUI-only: no console window alongside the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ssh started this executable as SSH_ASKPASS: answer and exit, no window.
    if let Some(code) = simter_lib::askpass::helper_main() {
        std::process::exit(code);
    }
    simter_lib::run()
}
