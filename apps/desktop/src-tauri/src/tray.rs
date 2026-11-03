//! Icône et menu de la zone de notification.

use crate::{quit, show_main_window, AppState};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const TRAY_ID: &str = "ndm-tray";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Ouvrir Nova Download Manager", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume_all", "Tout reprendre", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause_all", "Tout mettre en pause", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep, &resume, &pause, &sep2, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Nova Download Manager")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "resume_all" => {
                if let Some(s) = app.try_state::<AppState>() {
                    s.engine.resume_all();
                }
            }
            "pause_all" => {
                if let Some(s) = app.try_state::<AppState>() {
                    s.engine.pause_all();
                }
            }
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn update_tooltip(app: &AppHandle, active: u32, total_speed: u64) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let text = if active == 0 {
            "Nova Download Manager".to_string()
        } else {
            format!("Nova Download Manager — {active} en cours, {}", format_speed(total_speed))
        };
        let _ = tray.set_tooltip(Some(text));
    }
}

fn format_speed(bps: u64) -> String {
    const UNITS: [&str; 4] = ["o/s", "Ko/s", "Mo/s", "Go/s"];
    let mut v = bps as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{v:.0} {}", UNITS[i]) } else { format!("{v:.1} {}", UNITS[i]) }
}
