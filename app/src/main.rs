// 正式版不顯示黑色主控台視窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod printer;
mod storage;

use commands::AppState;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

/// 把主視窗叫到前面（從右下角常駐、或重複開啟 exe 時）
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn main() {
    let data_dir = storage::pick_data_dir();
    let (db, init_error) = match storage::open(&data_dir) {
        Ok(db) => {
            let _ = storage::daily_backup(&db, &data_dir);
            (Some(db), None)
        }
        Err(e) => (None, Some(format!("{:#}", e))),
    };

    tauri::Builder::default()
        // 只允許開一個，重複開啟 exe 時把常駐中的那一個叫出來
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        // 按視窗的 ✕ 不結束程式，縮到右下角繼續提供手機同步
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .manage(AppState {
            db: Arc::new(Mutex::new(db)),
            data_dir,
            init_error,
            sync_server: Mutex::new(None),
            sync_error: Mutex::new(None),
        })
        .setup(|app| {
            // 啟動手機同步服務（設定裡可以關閉或改連接埠）
            commands::start_sync_server(&app.state::<AppState>());

            // 右下角常駐圖示：左鍵開啟，右鍵選單「開啟／結束程式」
            let open = MenuItem::with_id(app, "open", "開啟出貨系統", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "結束程式（手機將無法同步）", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("泓昇出貨系統（手機同步中）")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id().as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_settings,
            commands::save_settings,
            commands::list_printers,
            commands::check_text,
            commands::list_customers,
            commands::save_customer,
            commands::delete_customer,
            commands::delete_product,
            commands::customer_balances,
            commands::list_products,
            commands::save_product,
            commands::list_customer_prices,
            commands::set_customer_price,
            commands::suggest_price,
            commands::list_documents,
            commands::get_document,
            commands::save_document,
            commands::delete_document,
            commands::mark_issued,
            commands::add_payment,
            commands::list_payments,
            commands::delete_payment,
            commands::statement_period,
            commands::preview_statement,
            commands::create_statement,
            commands::list_statements,
            commands::void_statement,
            commands::list_manual,
            commands::get_manual,
            commands::save_manual,
            commands::delete_manual,
            commands::mark_manual_issued,
            commands::render_manual,
            commands::print_manual,
            commands::render_document,
            commands::render_statement,
            commands::render_test_page,
            commands::print_document,
            commands::print_statement,
            commands::print_test_page,
            commands::backup_now,
            commands::sync_status,
            commands::restart_sync_server,
            commands::create_pairing_code,
            commands::list_devices,
            commands::revoke_device,
            commands::rename_device,
            commands::list_sync_conflicts,
            commands::resolve_sync_conflict,
            commands::possible_duplicates,
            commands::confirm_customer,
            commands::merge_customers,
        ])
        .run(tauri::generate_context!())
        .expect("程式啟動失敗");
}
