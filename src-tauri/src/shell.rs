//! 管理预创建窗口、托盘与保存确认握手；通知只是提示，待办动作保存在原生状态中。

use std::sync::{atomic::{AtomicBool, Ordering}, mpsc, Mutex};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow, WebviewWindowBuilder,
    menu::{Menu, MenuItem}, tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent}};
use crate::{error::AppError, hotkey::{self, HotkeyInfo, HotkeyState}, storage::{StorageInfo, StorageState}};

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind { Hide, Exit }

#[derive(Clone, Serialize)]
pub struct ShellAction {
    pub id: u32,
    pub kind: ActionKind,
}

#[derive(Default)]
struct ShellData {
    capture_ready: bool,
    // 初始化错误独立保留，不能被后续调起或快捷键提示覆盖。
    capture_error: Option<AppError>,
    next_action_id: u32,
    pending_action: Option<ShellAction>,
    last_error: Option<AppError>,
    last_position: Option<PhysicalPosition<i32>>,
}

#[derive(Default)]
pub struct ShellState {
    // UI 线程变更窗口动作；异步 IPC 只读取快照，不在持锁时等待任务。
    data: Mutex<ShellData>,
    tray: Mutex<Option<TrayIcon>>,
    exiting: AtomicBool,
}

#[derive(Serialize)]
pub struct RuntimeSnapshot {
    pub capture_ready: bool,
    pub capture_error: Option<AppError>,
    pub pending_action: Option<ShellAction>,
    pub shell_error: Option<AppError>,
    pub storage: StorageInfo,
    pub hotkey: HotkeyInfo,
}

/// 将短时系统操作移到 UI 线程，不让工作线程持壳层锁等待 UI。
pub async fn on_main_thread<T, F>(app: AppHandle, operation: F) -> Result<T, AppError>
where T: Send + 'static, F: FnOnce(&AppHandle) -> Result<T, AppError> + Send + 'static {
    let (sender, receiver) = mpsc::sync_channel(1);
    let callback_app = app.clone();
    app.run_on_main_thread(move || { let _ = sender.send(operation(&callback_app)); })
        .map_err(|_| AppError::new("UI_DISPATCH", "系统操作无法调度到主线程。"))?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv()
        .map_err(|_| AppError::new("UI_DISPATCH", "系统操作未返回结果。")))
        .await.map_err(|_| AppError::new("UI_DISPATCH", "系统操作等待任务失败。"))?
        ?
}

/// 状态保留于本进程原生内存；通知丢失时页面可主动查询，不承载用户正文。
pub fn notify_status(app: &AppHandle) {
    let _ = app.emit("runtime-changed", ());
}

/// 保留最近系统错误并显示主窗口；不覆盖独立的 capture 初始化失败原因。
pub fn report_error(app: &AppHandle, error: AppError) {
    if let Ok(mut data) = app.state::<ShellState>().data.lock() { data.last_error = Some(error); }
    notify_status(app);
    // 失败后保留可见入口，不能把常驻错误藏在终端日志里。
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 标记 capture 初始化失败并保留原因；只有后续成功载入才能清除此错误。
pub fn report_capture_failure(app: &AppHandle, error: AppError) {
    if let Ok(mut data) = app.state::<ShellState>().data.lock() {
        data.capture_ready = false;
        data.capture_error = Some(error.clone());
    }
    report_error(app, error);
}

/// 数据库初始化完成后一次性创建隐藏 capture；不通过隐藏窗口事件启动初始化。
pub fn create_capture(app: &AppHandle) -> Result<(), AppError> {
    if app.get_webview_window("capture").is_some() { return Ok(()); }
    let config = app.config().app.windows.iter().find(|window| window.label == "capture")
        .ok_or_else(|| AppError::new("CAPTURE_CONFIG", "快速记录窗口配置缺失。"))?;
    WebviewWindowBuilder::from_config(app, config)
        .and_then(|builder| builder.build())
        .map_err(|error| AppError::new("CAPTURE_CREATE", format!("快速记录窗口无法创建：{error}")))?;
    Ok(())
}

/// 恢复、显示并聚焦已有主窗口，供托盘和二次启动复用。
pub fn show_main(app: &AppHandle) -> Result<(), AppError> {
    let window = app.get_webview_window("main")
        .ok_or_else(|| AppError::new("MAIN_WINDOW", "主窗口不存在。"))?;
    window.unminimize().and_then(|_| window.show()).and_then(|_| window.set_focus())
        .map_err(|error| AppError::new("WINDOW_FOCUS", format!("主窗口无法显示或聚焦：{error}")))
}

/// 在鼠标所在屏幕定位首次显示；后续位置始终限制在当前可见工作区内。
fn position_capture(app: &AppHandle, window: &WebviewWindow) -> Result<(), AppError> {
    let previous = app.state::<ShellState>().data.lock()
        .map_err(|_| AppError::new("STATE_LOCK", "窗口位置状态不可读取。"))?.last_position;
    let point = if let Some(position) = previous { (position.x as f64, position.y as f64) }
        else {
            let cursor = window.cursor_position().map_err(|error| AppError::new("MONITOR", format!("鼠标位置无法读取：{error}")))?;
            (cursor.x, cursor.y)
        };
    let monitor = match window.monitor_from_point(point.0, point.1)
        .map_err(|error| AppError::new("MONITOR", format!("显示器无法读取：{error}")))? {
        Some(monitor) => Some(monitor),
        None => window.primary_monitor()
            .map_err(|error| AppError::new("MONITOR", format!("主显示器无法读取：{error}")))?,
    }.ok_or_else(|| AppError::new("MONITOR", "没有可用于快速记录的显示器。"))?;
    let area = monitor.work_area();
    let size = window.outer_size().map_err(|error| AppError::new("WINDOW_POSITION", format!("窗口尺寸无法读取：{error}")))?;
    let spare_x = (area.size.width as i64 - size.width as i64).max(0);
    let spare_y = (area.size.height as i64 - size.height as i64).max(0);
    let x = previous.map(|position| position.x as i64).unwrap_or(area.position.x as i64 + spare_x / 2)
        .clamp(area.position.x as i64, area.position.x as i64 + spare_x);
    let y = previous.map(|position| position.y as i64).unwrap_or(area.position.y as i64 + spare_y / 4)
        .clamp(area.position.y as i64, area.position.y as i64 + spare_y);
    window.set_position(PhysicalPosition::new(x as i32, y as i32))
        .map_err(|error| AppError::new("WINDOW_POSITION", format!("快速记录窗口无法定位：{error}")))
}

/// 只显示已载入草稿的预创建窗口；系统焦点与输入法效果仍须真机确认。
pub fn show_capture(app: &AppHandle) -> Result<(), AppError> {
    let ready = app.state::<ShellState>().data.lock()
        .map_err(|_| AppError::new("STATE_LOCK", "窗口状态不可读取。"))?.capture_ready;
    if !ready { return Err(AppError::new("CAPTURE_NOT_READY", "快速记录尚未就绪，请在主窗口查看初始化状态或错误。")); }
    let window = app.get_webview_window("capture")
        .ok_or_else(|| AppError::new("CAPTURE_WINDOW", "快速记录窗口不存在。"))?;
    if !window.is_visible().map_err(|error| AppError::new("WINDOW_STATE", format!("窗口状态无法读取：{error}")))? {
        position_capture(app, &window)?;
    }
    window.unminimize().and_then(|_| window.show()).and_then(|_| window.set_focus())
        .map_err(|error| AppError::new("WINDOW_FOCUS", format!("快速记录无法显示或聚焦：{error}")))
}

/// 记录待隐藏或退出动作；退出会先显示 capture，由 DOM 焦点同步触发保存确认。
pub fn request_action(app: &AppHandle, kind: ActionKind) -> Result<Option<ShellAction>, AppError> {
    let state = app.state::<ShellState>();
    let action = {
        let mut data = state.data.lock().map_err(|_| AppError::new("STATE_LOCK", "窗口动作不可更新。"))?;
        if !data.capture_ready {
            // 未就绪的编辑器尚未允许输入，此时退出不会丢失内存中的用户输入。
            if kind == ActionKind::Exit {
                state.exiting.store(true, Ordering::Relaxed);
                drop(data);
                app.exit(0);
                return Ok(None);
            }
            return Err(AppError::new("CAPTURE_NOT_READY", "快速记录尚未就绪。"));
        }
        if let Some(existing) = data.pending_action.as_ref()
            .filter(|action| action.kind == ActionKind::Exit || kind == ActionKind::Hide).cloned() {
            // 重试沿用同一动作 ID，但不能跳过锁外的唤醒和通知。
            existing
        } else {
            data.next_action_id = data.next_action_id.wrapping_add(1).max(1);
            let action = ShellAction { id: data.next_action_id, kind };
            data.pending_action = Some(action.clone());
            action
        }
    };
    show_capture(app)?;
    // 可见窗口收到提示后查询原生状态；隐藏窗口调起不依赖该通知。
    let _ = app.emit_to("capture", "shell-intent", ());
    notify_status(app);
    Ok(Some(action))
}

/// 全局快捷键切换窗口；可见时请求保存后隐藏，隐藏时显示预创建窗口。
pub fn toggle_capture(app: &AppHandle) -> Result<(), AppError> {
    let visible = app.get_webview_window("capture").map(|window| window.is_visible())
        .transpose().map_err(|error| AppError::new("WINDOW_STATE", format!("窗口状态无法读取：{error}")))?
        .unwrap_or(false);
    if visible { request_action(app, ActionKind::Hide).map(|_| ()) } else { show_capture(app) }
}

/// 按当前暂停状态构造托盘菜单，菜单只触发系统操作，不承载记录规则。
fn tray_menu(app: &AppHandle) -> Result<Menu, AppError> {
    let info = app.state::<HotkeyState>().snapshot()?;
    let items = [
        MenuItem::with_id(app, "capture", "快速记录", true, None::<&str>),
        MenuItem::with_id(app, "main", "主窗口 / 换键", true, None::<&str>),
        MenuItem::with_id(app, "pause", if info.paused { "恢复快捷键" } else { "暂停快捷键" }, true, None::<&str>),
        MenuItem::with_id(app, "quit", "保存草稿并退出", true, None::<&str>),
    ];
    let items: Vec<MenuItem> = items.into_iter().collect::<Result<_, _>>()
        .map_err(|error| AppError::new("TRAY_MENU", format!("托盘菜单无法创建：{error}")))?;
    let references: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = items.iter()
        .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>).collect();
    Menu::with_items(app, &references).map_err(|error| AppError::new("TRAY_MENU", format!("托盘菜单无法创建：{error}")))
}

/// 更新托盘提示，快捷键失败时用户仍可通过主窗口与托盘调起记录。
pub fn refresh_tray(app: &AppHandle) -> Result<(), AppError> {
    let tray = app.state::<ShellState>().tray.lock()
        .map_err(|_| AppError::new("STATE_LOCK", "托盘状态不可读取。"))?.clone();
    if let Some(tray) = tray {
        let info = app.state::<HotkeyState>().snapshot()?;
        let tooltip = if info.error.is_some() { "MirrorMind · 快捷键错误，请打开主窗口" }
            else if info.paused { "MirrorMind · 快捷键已暂停" } else { "MirrorMind · 快速记录" };
        tray.set_menu(Some(tray_menu(app)?)).and_then(|_| tray.set_tooltip(Some(tooltip)))
            .map_err(|error| AppError::new("TRAY_MENU", format!("托盘状态无法更新：{error}")))?;
    }
    Ok(())
}

/// 创建应用级托盘资源与明确退出入口；关闭主窗口不销毁该资源。
pub fn initialize_tray(app: &AppHandle) -> Result<(), AppError> {
    let mut rgba = vec![0_u8; 32 * 32 * 4];
    for y in 4..28_usize { for x in 6..26_usize {
        if x < 9 || x > 22 || y < 7 || y > 24 || (x >= 14 && x <= 17) {
            let offset = (y * 32 + x) * 4;
            rgba[offset..offset + 4].copy_from_slice(&[38, 107, 88, 255]);
        }
    } }
    let tray = TrayIconBuilder::with_id("mirrormind")
        .icon(tauri::image::Image::new_owned(rgba, 32, 32))
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&tray_menu(app)?).show_menu_on_left_click(false)
        .tooltip("MirrorMind · 快速记录")
        .on_menu_event(|app, event| {
            let result = match event.id.as_ref() {
                "capture" => show_capture(app),
                "main" => show_main(app),
                "pause" => app.state::<HotkeyState>().snapshot()
                    .and_then(|info| hotkey::set_paused_native(app, !info.paused)).map(|_| ()),
                "quit" => request_action(app, ActionKind::Exit).map(|_| ()),
                _ => Ok(()),
            };
            if let Err(error) = result { report_error(app, error); }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) {
                if let Err(error) = show_main(tray.app_handle()) { report_error(tray.app_handle(), error); }
            }
        }).build(app).map_err(|error| AppError::new("TRAY_CREATE", format!("托盘无法创建，主窗口将保留：{error}")))?;
    *app.state::<ShellState>().tray.lock().map_err(|_| AppError::new("STATE_LOCK", "托盘状态不可更新。"))? = Some(tray);
    Ok(())
}

/// 主窗口关闭只隐藏；托盘不可用时保留窗口，避免失去所有常驻入口。
pub fn hide_main(app: &AppHandle) -> Result<(), AppError> {
    let has_tray = app.state::<ShellState>().tray.lock()
        .map_err(|_| AppError::new("STATE_LOCK", "托盘状态不可读取。"))?.is_some();
    if !has_tray { return Err(AppError::new("TRAY_UNAVAILABLE", "托盘不可用，主窗口不能隐藏。请使用保存草稿并退出。")); }
    app.get_webview_window("main").ok_or_else(|| AppError::new("MAIN_WINDOW", "主窗口不存在。"))?
        .hide().map_err(|error| AppError::new("WINDOW_HIDE", format!("主窗口无法隐藏：{error}")))
}

/// 判断是否已完成正常退出确认，避免退出事件被再次转为保存请求。
pub fn exit_confirmed(app: &AppHandle) -> bool {
    app.state::<ShellState>().exiting.load(Ordering::Relaxed)
}

/// 两个本地窗口均可查询状态；不依赖状态广播保证一致性。
#[tauri::command]
pub fn runtime_status(window: WebviewWindow, app: AppHandle) -> Result<RuntimeSnapshot, AppError> {
    if window.label() != "capture" && window.label() != "main" { return Err(AppError::new("FORBIDDEN", "该窗口不能读取应用状态。")); }
    let (capture_ready, capture_error, pending_action, shell_error) = {
        let state = app.state::<ShellState>();
        let data = state.data.lock().map_err(|_| AppError::new("STATE_LOCK", "窗口状态不可读取。"))?;
        (data.capture_ready, data.capture_error.clone(), data.pending_action.clone(), data.last_error.clone())
    };
    Ok(RuntimeSnapshot { capture_ready, capture_error, pending_action, shell_error,
        storage: app.state::<StorageState>().snapshot()?, hotkey: app.state::<HotkeyState>().snapshot()? })
}

/// capture 完成草稿载入后确认可输入；未收到确认前不会显示空编辑器。
#[tauri::command]
pub fn capture_ready(window: WebviewWindow, app: AppHandle, failure: Option<String>) -> Result<(), AppError> {
    if window.label() != "capture" { return Err(AppError::new("FORBIDDEN", "只有快速记录窗口可以确认就绪。")); }
    if let Some(message) = failure {
        report_capture_failure(&app, AppError::new("CAPTURE_INIT", message.chars().take(500).collect::<String>()));
        return Ok(());
    }
    if !app.state::<StorageState>().snapshot()?.ready { return Err(AppError::new("DB_NOT_READY", "数据库尚未就绪。")); }
    {
        let state = app.state::<ShellState>();
        let mut data = state.data.lock().map_err(|_| AppError::new("STATE_LOCK", "窗口状态不可更新。"))?;
        data.capture_ready = true;
        data.capture_error = None;
    }
    notify_status(&app);
    Ok(())
}

/// 从主窗口或 capture 调起已预加载的快速记录。
#[tauri::command]
pub async fn open_capture(window: WebviewWindow, app: AppHandle) -> Result<(), AppError> {
    if window.label() != "main" && window.label() != "capture" { return Err(AppError::new("FORBIDDEN", "该窗口不能调起快速记录。")); }
    on_main_thread(app, show_capture).await
}

/// capture 发起保留草稿并隐藏；返回动作 ID，前端保存后再确认，不依赖通知送达。
#[tauri::command]
pub async fn request_hide(window: WebviewWindow, app: AppHandle) -> Result<Option<ShellAction>, AppError> {
    if window.label() != "capture" { return Err(AppError::new("FORBIDDEN", "只有快速记录窗口可以请求隐藏。")); }
    on_main_thread(app, |app| request_action(app, ActionKind::Hide)).await
}

/// 用户明确退出后先唤醒 capture 保存；未初始化编辑器时可直接退出。
#[tauri::command]
pub async fn request_exit(window: WebviewWindow, app: AppHandle) -> Result<(), AppError> {
    if window.label() != "main" { return Err(AppError::new("FORBIDDEN", "只有主窗口可以请求退出。")); }
    on_main_thread(app, |app| request_action(app, ActionKind::Exit).map(|_| ())).await
}

/// 只接收 capture 对当前动作的保存确认；旧动作不隐藏窗口，也不退出应用。
#[tauri::command]
pub async fn finish_shell_action(window: WebviewWindow, app: AppHandle, action_id: u32) -> Result<bool, AppError> {
    if window.label() != "capture" { return Err(AppError::new("FORBIDDEN", "只有快速记录窗口可以确认保存动作。")); }
    on_main_thread(app, move |app| {
        let state = app.state::<ShellState>();
        let mut data = state.data.lock().map_err(|_| AppError::new("STATE_LOCK", "窗口动作不可更新。"))?;
        let Some(action) = data.pending_action.clone().filter(|action| action.id == action_id) else { return Ok(false); };
        match action.kind {
            ActionKind::Hide => {
                let window = app.get_webview_window("capture").ok_or_else(|| AppError::new("CAPTURE_WINDOW", "快速记录窗口不存在。"))?;
                data.last_position = window.outer_position().ok();
                window.hide().map_err(|error| AppError::new("WINDOW_HIDE", format!("草稿已保存，但窗口无法隐藏：{error}")))?;
                data.pending_action = None;
                drop(data);
                notify_status(app);
            }
            ActionKind::Exit => {
                data.pending_action = None;
                state.exiting.store(true, Ordering::Relaxed);
                drop(data);
                app.exit(0);
            }
        }
        Ok(true)
    }).await
}
