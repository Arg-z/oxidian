// Mencegah window console tambahan di Windows saat build release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    oxidian_app_lib::run()
}
