// Pas de fenêtre console en version finale sous Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ndm_desktop_lib::run()
}
