// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--apply-update") {
        match args.next() {
            Some(target) => memtools_lib::updater::apply_update(std::path::Path::new(&target)),
            None => std::process::exit(2),
        }
    }
    memtools_lib::run()
}
