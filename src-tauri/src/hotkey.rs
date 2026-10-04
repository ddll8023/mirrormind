//! 统一管理全局快捷键注册、可见错误和本次运行内的换键与暂停状态。

use std::{str::FromStr, sync::{atomic::{AtomicBool, Ordering}, Mutex}};
use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use crate::{error::AppError, shell};

#[derive(Clone, Serialize)]
pub struct HotkeyInfo {
    pub shortcut: String,
    pub paused: bool,
    pub registered: bool,
    pub error: Option<AppError>,
}

pub struct HotkeyState {
    // 变更均在 UI 线程串行执行；状态查询仅在短临界区读取快照。
    info: Mutex<HotkeyInfo>,
    pressed: AtomicBool,
}

impl Default for HotkeyState {
    /// 构造平台默认组合键与未注册状态，不在状态构造期间调用系统注册。
    fn default() -> Self {
        let shortcut = if cfg!(target_os = "macos") { "Super+Shift+Space" } else { "Control+Shift+Space" };
        Self {
            info: Mutex::new(HotkeyInfo { shortcut: shortcut.into(), paused: false, registered: false, error: None }),
            pressed: AtomicBool::new(false),
        }
    }
}

impl HotkeyState {
    /// 返回实际注册与暂停状态的快照，供窗口和托盘使用同一事实来源。
    pub fn snapshot(&self) -> Result<HotkeyInfo, AppError> {
        self.info.lock().map(|info| info.clone())
            .map_err(|_| AppError::new("STATE_LOCK", "快捷键状态不可读取。"))
    }
}

/// 校验组合键长度与系统解析格式；格式有效不代表未被其他程序占用。
fn parse(shortcut: &str) -> Result<Shortcut, AppError> {
    if shortcut.is_empty() || shortcut.len() > 128 {
        return Err(AppError::new("INVALID_SHORTCUT", "请输入合法的组合键，例如 Control+Shift+Space。"));
    }
    Shortcut::from_str(shortcut)
        .map_err(|_| AppError::new("INVALID_SHORTCUT", "组合键格式不正确；macOS 的 Command 可写为 Super。"))
}

/// 尝试注册默认键；失败保留入口与可查询错误，不把冲突解释为特定程序占用。
pub fn initialize(app: &AppHandle) -> Result<(), AppError> {
    let state = app.state::<HotkeyState>();
    let mut info = state.info.lock().map_err(|_| AppError::new("STATE_LOCK", "快捷键状态不可更新。"))?;
    let shortcut = parse(&info.shortcut)?;
    match app.global_shortcut().register(shortcut) {
        Ok(()) => { info.registered = true; Ok(()) }
        Err(_) => {
            let error = AppError::new("HOTKEY_REGISTER", "默认快捷键注册失败，可能已被占用或被系统拒绝。请在主窗口换键；无法判断占用程序。" );
            info.error = Some(error.clone());
            Err(error)
        }
    }
}

/// 只响应一次按下；释放后解除防重，避免按住组合键反复切换窗口。
pub fn handle_event(app: &AppHandle, event: ShortcutEvent) {
    let state = app.state::<HotkeyState>();
    if event.state == ShortcutState::Released {
        state.pressed.store(false, Ordering::Relaxed);
        return;
    }
    if state.pressed.swap(true, Ordering::Relaxed) { return; }
    // 插件回调仍持有注册表锁，只调度 UI 操作，避免换键与窗口操作相互等待。
    let callback_app = app.clone();
    if app.run_on_main_thread(move || {
        if let Err(error) = shell::toggle_capture(&callback_app) { shell::report_error(&callback_app, error); }
    }).is_err() {
        state.pressed.store(false, Ordering::Relaxed);
    }
}

/// 先注册新键，再注销旧键；新键失败不改变正在使用的旧键。
fn change_shortcut(app: &AppHandle, requested: String) -> Result<HotkeyInfo, AppError> {
    let requested = requested.trim().to_owned();
    let new_shortcut = parse(&requested)?;
    let state = app.state::<HotkeyState>();
    let mut info = state.info.lock().map_err(|_| AppError::new("STATE_LOCK", "快捷键状态不可更新。"))?;
    let old_shortcut = parse(&info.shortcut)?;
    if info.registered && old_shortcut.id() == new_shortcut.id() {
        info.error = None;
        return Ok(info.clone());
    }
    if !info.paused {
        if app.global_shortcut().register(new_shortcut).is_err() {
            let error = AppError::new("HOTKEY_REGISTER", "新快捷键注册失败，仍保留旧配置。可能已被占用或被系统拒绝；无法判断占用程序。" );
            info.error = Some(error.clone());
            return Err(error);
        }
        if info.registered && app.global_shortcut().unregister(old_shortcut).is_err() {
            let restored = app.global_shortcut().unregister(new_shortcut).is_ok();
            let error = AppError::new("HOTKEY_REBIND", if restored {
                "旧快捷键无法注销，已撤销新键注册。"
            } else {
                "换键回退失败，可能同时存在两个已注册键。请暂停快捷键后重试。"
            });
            info.error = Some(error.clone());
            return Err(error);
        }
        info.registered = true;
    }
    info.shortcut = requested;
    info.error = None;
    state.pressed.store(false, Ordering::Relaxed);
    Ok(info.clone())
}

/// 暂停只注销本插件管理的快捷键；恢复失败保持暂停并反馈错误。
pub fn set_paused_native(app: &AppHandle, paused: bool) -> Result<HotkeyInfo, AppError> {
    let state = app.state::<HotkeyState>();
    let result = {
        let mut info = state.info.lock().map_err(|_| AppError::new("STATE_LOCK", "快捷键状态不可更新。"))?;
        if !paused && info.registered {
            info.error = None;
            Ok(info.clone())
        } else {
            let operation = if paused { app.global_shortcut().unregister_all() }
                else { app.global_shortcut().register(parse(&info.shortcut)?) };
            if operation.is_err() {
                let error = AppError::new("HOTKEY_PAUSE", "快捷键状态切换失败，原状态未改变。请换键或重试。" );
                info.error = Some(error.clone());
                Err(error)
            } else {
                info.paused = paused;
                info.registered = !paused;
                info.error = None;
                state.pressed.store(false, Ordering::Relaxed);
                Ok(info.clone())
            }
        }
    };
    if let Err(error) = shell::refresh_tray(app) { shell::report_error(app, error); }
    shell::notify_status(app);
    result
}

/// 主窗口修改本次运行的快捷键；不允许 capture 或未来其他窗口修改注册。
#[tauri::command]
pub async fn set_shortcut(window: WebviewWindow, app: AppHandle, shortcut: String) -> Result<HotkeyInfo, AppError> {
    if window.label() != "main" { return Err(AppError::new("FORBIDDEN", "只有主窗口可以修改快捷键。")); }
    shell::on_main_thread(app, move |app| {
        let result = change_shortcut(app, shortcut);
        if let Err(error) = shell::refresh_tray(app) { shell::report_error(app, error); }
        shell::notify_status(app);
        result
    }).await
}

/// 主窗口暂停或恢复快捷键，不影响草稿与存储。
#[tauri::command]
pub async fn set_paused(window: WebviewWindow, app: AppHandle, paused: bool) -> Result<HotkeyInfo, AppError> {
    if window.label() != "main" { return Err(AppError::new("FORBIDDEN", "只有主窗口可以修改暂停状态。")); }
    shell::on_main_thread(app, move |app| set_paused_native(app, paused)).await
}
