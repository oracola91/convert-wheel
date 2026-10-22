#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod formats;
mod trigger;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;

/// The wheel got a drop. Proof step: report what landed where; converting comes next.
#[tauri::command]
fn wheel_dropped(app: tauri::AppHandle, paths: Vec<String>, format: Option<String>) {
    match &format {
        Some(f) => eprintln!("convert-wheel: {} file(s) -> {f}: {paths:?}", paths.len()),
        None => eprintln!("convert-wheel: drop cancelled: {paths:?}"),
    }
    trigger::hide_wheel(&app);
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // Menu bar style app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = MenuItem::with_id(app, "quit", "Quit Convert Wheel", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Convert Wheel: hold Shift while dragging a file")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            trigger::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![wheel_dropped])
        .run(tauri::generate_context!())
        .expect("error while running convert-wheel");
}
