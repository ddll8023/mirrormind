//! 装配 MirrorMind 桌面壳；窗口与系统行为归 Rust，记录规则与业务 SQL 归 TS。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod error;
mod hotkey;
mod migrations;
mod shell;
mod storage;

use tauri::{Manager, RunEvent, WindowEvent};

/// 在系统主线程装配常驻壳；启动构建失败时先显示原生错误提示再退出。
fn main() {
    let application = tauri::Builder::default()
        // 单实例插件优先注册，二次启动只唤醒原进程，不打开第二个数据库。
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Err(error) = shell::show_main(app) { shell::report_error(app, error); }
        }))
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _, event| hotkey::handle_event(app, event)).build())
        .manage(storage::StorageState::default())
        .manage(shell::ShellState::default())
        .manage(hotkey::HotkeyState::default())
        .invoke_handler(tauri::generate_handler![
            shell::runtime_status, shell::capture_ready, shell::open_capture,
            shell::request_hide, shell::request_exit, shell::finish_shell_action,
            hotkey::set_shortcut, hotkey::set_paused, storage::db_transaction,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            if let Err(error) = shell::initialize_tray(app.handle()) { shell::report_error(app.handle(), error); }
            if let Err(error) = hotkey::initialize(app.handle()) { shell::report_error(app.handle(), error); }
            if let Err(error) = shell::refresh_tray(app.handle()) { shell::report_error(app.handle(), error); }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let initialized = storage::initialize(&handle).await;
                let result = shell::on_main_thread(handle, move |app| {
                    match initialized {
                        Ok(()) => {
                            if let Err(error) = shell::create_capture(app) { shell::report_capture_failure(app, error); }
                            shell::notify_status(app);
                            Ok(())
                        }
                        Err(error) => { shell::report_error(app, error); Ok(()) }
                    }
                }).await;
                // 原生错误保持在状态中；初始化不依赖隐藏 WebView 接收广播。
                if let Err(error) = result {
                    // 该分支通常为 UI 调度失败；此时至少保留终端诊断，不输出正文。
                    eprintln!("MirrorMind 初始化失败：{error}");
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle();
                let result = if window.label() == "main" { shell::hide_main(app) }
                    else if window.label() == "capture" { shell::request_action(app, shell::ActionKind::Hide).map(|_| ()) }
                    else { Ok(()) };
                if let Err(error) = result { shell::report_error(app, error); }
            }
        })
        .build(tauri::generate_context!());
    match application {
        Ok(application) => application.run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                if !shell::exit_confirmed(app) {
                    api.prevent_exit();
                    if let Err(error) = shell::request_action(app, shell::ActionKind::Exit) { shell::report_error(app, error); }
                }
            }
        }),
        Err(error) => {
            eprintln!("MirrorMind 桌面壳无法启动：{error}");
            // 此时不能依赖 WebView 或主窗口；便携版双击启动也必须看见失败原因。
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                rfd::MessageDialog::new()
                    .set_title("MirrorMind 启动失败")
                    .set_description(format!("MirrorMind 无法启动。\n\n{error}\n\n请检查运行环境。关闭此提示后程序将退出。"))
                    .set_level(rfd::MessageLevel::Error)
                    .show();
            }
            std::process::exit(1);
        }
    }
}
