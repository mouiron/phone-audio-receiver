use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(not(debug_assertions))]
use std::ffi::OsString;
#[cfg(not(debug_assertions))]
use std::os::windows::process::CommandExt;
#[cfg(not(debug_assertions))]
use std::process::Command;

use bluetooth_phone_audio_receiver_core::{bt, config, i18n};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_notification::NotificationExt;
use windows::core::{w, PCWSTR};
use windows::Media::Audio::AudioPlaybackConnection;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
};

const STARTUP_RUN_KEY: windows::core::PCWSTR =
    windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const STARTUP_VALUE_NAME: windows::core::PCWSTR = windows::core::w!("PhoneAudioReceiver");
const LEGACY_STARTUP_VALUE_NAME: windows::core::PCWSTR =
    windows::core::w!("BluetoothPhoneAudioReceiverTauri");
const TAURI_WINDOW_CLASS: PCWSTR = w!("PhoneAudioReceiverTauri");
const MAIN_WINDOW_TITLE_WIDE: PCWSTR = w!("Phone Audio Receiver");
const QUICK_WINDOW_TITLE: &str = "Phone Audio Receiver — Quick controls";
const QUICK_WINDOW_TITLE_WIDE: PCWSTR = w!("Phone Audio Receiver — Quick controls");
const TRAY_DEVICE_LIMIT: usize = 6;
#[cfg(not(debug_assertions))]
const PORTABLE_UNBLOCK_SCRIPT_NAME: &str = "unblock_downloaded_app.bat";
#[cfg(not(debug_assertions))]
const PORTABLE_UNBLOCK_SCRIPT: &[u8] = include_bytes!("../resources/unblock_downloaded_app.bat");
#[cfg(not(debug_assertions))]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(not(debug_assertions))]
const SINGLE_INSTANCE_MUTEX: PCWSTR = w!("Local\\PhoneAudioReceiver.SingleInstance");
// 配布版がタスクトレイで動作中でも `tauri dev` の検証プロセスを起動できるよう、
// dev版は単一起動の範囲を分離する。同じdev版の多重起動は引き続き防止する。
#[cfg(debug_assertions)]
const SINGLE_INSTANCE_MUTEX: PCWSTR = w!("Local\\PhoneAudioReceiver.Dev.SingleInstance");

#[cfg(debug_assertions)]
const CLOSE_STRATEGY: &str = "tauri-in-process-device-map-no-monitor-v12";

#[cfg(debug_assertions)]
fn build_profile() -> &'static str {
    "dev"
}

#[cfg(debug_assertions)]
fn executable_for_diagnostics() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("取得失敗: {error}"))
}

#[derive(Default)]
struct Runtime {
    devices: Vec<bt::DeviceEntry>,
    connections: HashMap<String, AudioPlaybackConnection>,
    connecting: HashMap<String, PendingConnection>,
    disconnect_timers: HashMap<String, DisconnectTimer>,
    next_request_id: u64,
    next_timer_id: u64,
}

struct PendingConnection {
    request_id: u64,
    cancel: Arc<AtomicBool>,
}

struct DisconnectTimer {
    timer_id: u64,
    cancel: Arc<AtomicBool>,
    deadline_epoch_seconds: u64,
}

struct AppState {
    runtime: Mutex<Runtime>,
    /// ×ボタンの処理中に設定ファイルを読み直さず、直近に確定した設定を使う。
    /// ウィンドウイベントとファイルI/Oが競合する余地をなくす。
    minimize_to_tray: AtomicBool,
    tray_actions: Mutex<HashMap<String, TrayAction>>,
    last_window_mode: Mutex<WindowMode>,
}

#[derive(Clone, Copy)]
enum WindowMode {
    Main,
    Quick,
}

#[derive(Clone)]
enum TrayAction {
    Connect(String),
    Disconnect(String),
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(Runtime::default()),
            minimize_to_tray: AtomicBool::new(config::load().minimize_to_tray),
            tray_actions: Mutex::new(HashMap::new()),
            last_window_mode: Mutex::new(WindowMode::Main),
        }
    }
}

fn hide_window_for_tray<R: tauri::Runtime>(window: &tauri::Window<R>) {
    // Tauriの状態管理と、Windowsシェル（ピン留めされたタスクバー項目を含む）の
    // 可視状態の両方を更新する。後者を明示することで、特定のWebView2状態で
    // Tauriのhide()だけが表示状態を更新しないケースを回避する。
    if let Err(error) = window.hide() {
        config::append_log(&format!(
            "Tauri経由のタスクトレイ格納に失敗しました: {error}"
        ));
    }
    match window.hwnd() {
        Ok(hwnd) => {
            // Tauriはwindows 0.61系、アプリ本体はwindows 0.62系を使うため、
            // HWNDの内部ポインタだけを同じWin32ハンドル型へ移し替える。
            let native_hwnd = windows::Win32::Foundation::HWND(hwnd.0);
            unsafe {
                let _ = ShowWindow(native_hwnd, SW_HIDE);
            }
            config::append_log("タスクトレイへ格納しました。");
        }
        Err(error) => config::append_log(&format!(
            "Windowsネイティブのタスクトレイ格納に失敗しました: {error}"
        )),
    }
}

fn hide_webview_window<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, description: &str) {
    if let Err(error) = window.hide() {
        config::append_log(&format!("{description}を非表示にできませんでした: {error}"));
    }
    match window.hwnd() {
        Ok(hwnd) => {
            let native_hwnd = windows::Win32::Foundation::HWND(hwnd.0);
            unsafe {
                let _ = ShowWindow(native_hwnd, SW_HIDE);
            }
        }
        Err(error) => config::append_log(&format!(
            "{description}のWindowsハンドルを取得できませんでした: {error}"
        )),
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView {
    id: String,
    name: String,
    system_name: String,
    status: String,
    auto_connect: bool,
    disconnect_timer_deadline: Option<u64>,
    favorite: bool,
    tray_pinned: bool,
    order_index: Option<usize>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    devices: Vec<DeviceView>,
    settings: Settings,
    last_device_id: Option<String>,
    auto_connect_device_ids: Vec<String>,
    display_language: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    minimize_to_tray: bool,
    auto_connect: bool,
    connection_notifications: bool,
    launch_at_login: bool,
    start_minimized: bool,
    theme: String,
}

impl From<config::AppConfig> for Settings {
    fn from(value: config::AppConfig) -> Self {
        Self {
            minimize_to_tray: value.minimize_to_tray,
            auto_connect: value.auto_connect,
            connection_notifications: value.connection_notifications,
            launch_at_login: value.launch_at_login,
            start_minimized: value.start_minimized,
            theme: match value.theme {
                config::ThemePreference::Light => "light",
                config::ThemePreference::System => "system",
                config::ThemePreference::Dark => "dark",
            }
            .to_string(),
        }
    }
}

fn configured_device_name(saved: &config::AppConfig, device_id: &str, system_name: &str) -> String {
    saved
        .device_preferences
        .get(device_id)
        .map(|value| value.alias.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or(system_name)
        .to_string()
}

fn snapshot(state: &AppState) -> AppSnapshot {
    let saved = config::load();
    let runtime = state.runtime.lock().expect("runtime lock poisoned");
    let devices = runtime
        .devices
        .iter()
        .map(|device| {
            let preference = saved.device_preferences.get(&device.id);
            DeviceView {
                id: device.id.clone(),
                name: configured_device_name(&saved, &device.id, &device.name),
                system_name: device.name.clone(),
                status: if runtime.connections.contains_key(&device.id) {
                    "connected"
                } else if runtime.connecting.contains_key(&device.id) {
                    "connecting"
                } else {
                    "disconnected"
                }
                .to_string(),
                auto_connect: saved.auto_connect_device_ids.contains(&device.id),
                disconnect_timer_deadline: runtime
                    .disconnect_timers
                    .get(&device.id)
                    .map(|timer| timer.deadline_epoch_seconds),
                favorite: preference.is_some_and(|value| value.favorite),
                tray_pinned: preference.is_some_and(|value| value.tray_pinned),
                order_index: saved.device_order.iter().position(|id| id == &device.id),
            }
        })
        .collect();
    AppSnapshot {
        devices,
        last_device_id: saved.last_device_id.clone(),
        auto_connect_device_ids: saved.auto_connect_device_ids.clone(),
        display_language: match saved.display_language {
            Some(config::DisplayLanguage::Japanese) => "ja",
            Some(config::DisplayLanguage::English) => "en",
            None => match i18n::system_language() {
                i18n::Language::Japanese => "ja",
                i18n::Language::English => "en",
            },
        }
        .to_string(),
        settings: saved.into(),
    }
}

fn emit_snapshot(app: &AppHandle, state: &AppState) {
    let value = snapshot(state);
    let _ = app.emit("app-state-changed", &value);
    if let Err(error) = update_tray_menu(app, state, &value) {
        config::append_log(&format!(
            "タスクトレイメニューを更新できませんでした: {error}"
        ));
    }
}

fn sort_devices_by_name(devices: &mut [DeviceView]) {
    devices.sort_by(|left, right| {
        left.order_index
            .unwrap_or(usize::MAX)
            .cmp(&right.order_index.unwrap_or(usize::MAX))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
}

const DISCONNECT_TIMER_MINUTES: [u64; 4] = [15, 30, 60, 120];

fn cancel_disconnect_timer(runtime: &mut Runtime, device_id: &str) -> bool {
    if let Some(timer) = runtime.disconnect_timers.remove(device_id) {
        timer.cancel.store(true, Ordering::Release);
        true
    } else {
        false
    }
}

fn wait_for_timer(duration: Duration, cancel: &AtomicBool) -> bool {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        thread::sleep(remaining.min(Duration::from_secs(1)));
    }
    !cancel.load(Ordering::Acquire)
}

fn move_device_id(ids: &mut [String], device_id: &str, direction: &str) -> bool {
    let Some(index) = ids.iter().position(|id| id == device_id) else {
        return false;
    };
    let target = if direction == "up" {
        index.checked_sub(1)
    } else if direction == "down" && index + 1 < ids.len() {
        Some(index + 1)
    } else {
        None
    };
    if let Some(target) = target {
        ids.swap(index, target);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod device_sort_tests {
    use super::*;

    fn device(id: &str, name: &str, status: &str) -> DeviceView {
        DeviceView {
            id: id.to_string(),
            name: name.to_string(),
            system_name: name.to_string(),
            status: status.to_string(),
            auto_connect: false,
            disconnect_timer_deadline: None,
            favorite: false,
            tray_pinned: false,
            order_index: None,
        }
    }

    #[test]
    fn device_name_order_does_not_change_with_connection_status() {
        let mut devices = vec![
            device("b", "Phone B", "connected"),
            device("a", "Phone A", "disconnected"),
        ];
        sort_devices_by_name(&mut devices);
        assert_eq!(devices[0].id, "a");
        assert_eq!(devices[1].id, "b");

        devices[0].status = "connected".to_string();
        devices[1].status = "disconnected".to_string();
        sort_devices_by_name(&mut devices);
        assert_eq!(devices[0].id, "a");
        assert_eq!(devices[1].id, "b");
    }

    #[test]
    fn cancelling_one_disconnect_timer_keeps_the_other_device_timer() {
        let mut runtime = Runtime::default();
        for (timer_id, device_id) in [(1, "device-a"), (2, "device-b")] {
            runtime.disconnect_timers.insert(
                device_id.to_string(),
                DisconnectTimer {
                    timer_id,
                    cancel: Arc::new(AtomicBool::new(false)),
                    deadline_epoch_seconds: 1_000,
                },
            );
        }

        assert!(cancel_disconnect_timer(&mut runtime, "device-a"));
        assert!(!runtime.disconnect_timers.contains_key("device-a"));
        assert_eq!(runtime.disconnect_timers["device-b"].timer_id, 2);
        assert!(!runtime.disconnect_timers["device-b"]
            .cancel
            .load(Ordering::Acquire));
    }

    #[test]
    fn moving_a_device_changes_only_the_matching_id() {
        let mut ids = vec![
            "device-a".to_string(),
            "device-b".to_string(),
            "device-c".to_string(),
        ];

        assert!(move_device_id(&mut ids, "device-b", "up"));
        assert_eq!(ids, ["device-b", "device-a", "device-c"]);
        assert!(!move_device_id(&mut ids, "unknown", "down"));
        assert_eq!(ids, ["device-b", "device-a", "device-c"]);
    }
}

fn update_tray_menu(app: &AppHandle, state: &AppState, value: &AppSnapshot) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id("main") else {
        return Ok(());
    };
    let language = match value.display_language.as_str() {
        "ja" => i18n::Language::Japanese,
        _ => i18n::Language::English,
    };
    let connected = value
        .devices
        .iter()
        .filter(|device| device.status == "connected")
        .count();
    let mut devices = value.devices.clone();
    sort_devices_by_name(&mut devices);
    devices.sort_by_key(|device| {
        if device.tray_pinned {
            0
        } else if device.status != "disconnected" {
            1
        } else if device.favorite {
            2
        } else {
            3
        }
    });

    let menu = Menu::new(app)?;
    let summary_label = match language {
        i18n::Language::Japanese => format!("接続中: {connected}台"),
        i18n::Language::English => format!("Connected: {connected}"),
    };
    let summary = MenuItem::with_id(app, "tray-summary", summary_label, false, None::<&str>)?;
    menu.append(&summary)?;

    let quick = MenuItem::with_id(
        app,
        "quick",
        language.select("クイック操作", "Quick controls"),
        true,
        None::<&str>,
    )?;
    menu.append(&quick)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let mut actions = HashMap::new();
    for (index, device) in devices.iter().take(TRAY_DEVICE_LIMIT).enumerate() {
        let action_id = format!("tray-device-action-{index}");
        let label = match device.status.as_str() {
            "connected" => format!("✓ {}", device.name),
            "connecting" => format!("… {}", device.name),
            _ => format!("  {}", device.name),
        }
        .replace('&', "&&");
        let submenu = Submenu::with_id(app, format!("tray-device-{index}"), label, true)?;
        let action_label = match device.status.as_str() {
            "connected" => language.select("切断", "Disconnect"),
            "connecting" => language.select("中止", "Cancel"),
            _ => language.select("接続", "Connect"),
        };
        let action = MenuItem::with_id(app, &action_id, action_label, true, None::<&str>)?;
        submenu.append(&action)?;
        menu.append(&submenu)?;
        actions.insert(
            action_id,
            if device.status == "disconnected" {
                TrayAction::Connect(device.id.clone())
            } else {
                TrayAction::Disconnect(device.id.clone())
            },
        );
    }
    if devices.is_empty() {
        let empty = MenuItem::with_id(
            app,
            "tray-empty",
            language.select("端末が見つかっていません", "No devices found"),
            false,
            None::<&str>,
        )?;
        menu.append(&empty)?;
    } else if devices.len() > TRAY_DEVICE_LIMIT {
        let remaining = devices.len() - TRAY_DEVICE_LIMIT;
        let more_label = match language {
            i18n::Language::Japanese => format!("その他の端末…（{remaining}台）"),
            i18n::Language::English => format!("More devices… ({remaining})"),
        };
        let more = MenuItem::with_id(app, "more", more_label, true, None::<&str>)?;
        menu.append(&more)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let refresh = MenuItem::with_id(
        app,
        "refresh",
        language.select("再検索", "Refresh"),
        true,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(
        app,
        "show",
        language.select("アプリを開く", "Open app"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        language.select("終了", "Quit"),
        true,
        None::<&str>,
    )?;
    menu.append(&refresh)?;
    menu.append(&show)?;
    menu.append(&quit)?;
    if let Ok(mut stored) = state.tray_actions.lock() {
        *stored = actions;
    }
    tray.set_menu(Some(menu))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionEvent {
    device_id: String,
    name: String,
    status: String,
}

fn emit_connection_event(app: &AppHandle, device_id: String, name: String, status: &str) {
    let _ = app.emit(
        "connection-state-changed",
        ConnectionEvent {
            device_id,
            name,
            status: status.to_string(),
        },
    );
}

#[tauri::command]
fn app_snapshot(state: State<'_, Arc<AppState>>) -> AppSnapshot {
    snapshot(state.inner().as_ref())
}

#[tauri::command(async)]
fn refresh_devices(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<AppSnapshot, String> {
    // Minimal検証版と同じく、WinRTを明示的に初期化した専用MTAスレッドで列挙する。
    let worker = thread::spawn(|| {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        bt::enumerate_devices().map_err(|error| format!("{error:?}"))
    });
    let devices = worker
        .join()
        .map_err(|_| "デバイス検索スレッドが異常終了しました。".to_string())??;
    {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "状態のロックに失敗しました")?;
        let mut visible_devices = devices;
        let retained_ids: Vec<String> = runtime
            .connections
            .keys()
            .chain(runtime.connecting.keys())
            .cloned()
            .collect();
        for device_id in retained_ids {
            if visible_devices.iter().any(|device| device.id == device_id) {
                continue;
            }
            if let Some(device) = runtime.devices.iter().find(|device| device.id == device_id) {
                visible_devices.push(device.clone());
            }
        }
        runtime.devices = visible_devices;
    }
    config::append_log("デバイス一覧を更新しました。");
    let value = snapshot(state.inner().as_ref());
    let _ = app.emit("app-state-changed", &value);
    if let Err(error) = update_tray_menu(&app, state.inner().as_ref(), &value) {
        config::append_log(&format!(
            "タスクトレイメニューを更新できませんでした: {error}"
        ));
    }
    Ok(value)
}

#[tauri::command(async)]
fn connect_device(
    app: AppHandle,
    device_id: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let (name, request_id, cancel) = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "状態のロックに失敗しました")?;
        if runtime.connections.contains_key(&device_id)
            || runtime.connecting.contains_key(&device_id)
        {
            return Ok(());
        }
        let name = runtime
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .map(|device| device.name.clone())
            .ok_or_else(|| "デバイスが一覧にありません。再検索してください。".to_string())?;
        runtime.next_request_id += 1;
        let request_id = runtime.next_request_id;
        let cancel = Arc::new(AtomicBool::new(false));
        runtime.connecting.insert(
            device_id.clone(),
            PendingConnection {
                request_id,
                cancel: Arc::clone(&cancel),
            },
        );
        (name, request_id, cancel)
    };

    config::append_log(&format!("接続試行開始: {name} ({device_id})"));
    emit_snapshot(&app, state.inner().as_ref());
    let worker_device_id = device_id.clone();
    let progress_app = app.clone();
    let progress_state = Arc::downgrade(state.inner());
    let worker = thread::spawn(move || {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let id_for_progress = worker_device_id.clone();
        bt::connect_with_retry(&worker_device_id, &cancel, move |message| {
            config::append_log(&message);
            let is_current_request = progress_state
                .upgrade()
                .and_then(|state| {
                    state.runtime.lock().ok().map(|runtime| {
                        runtime
                            .connecting
                            .get(&id_for_progress)
                            .is_some_and(|pending| pending.request_id == request_id)
                    })
                })
                .unwrap_or(false);
            if is_current_request {
                let _ = progress_app.emit(
                    "connection-progress",
                    ProgressEvent {
                        device_id: id_for_progress.clone(),
                        message,
                    },
                );
            }
        })
        .map_err(|error| format!("{error:?}"))
    });
    let result = match worker.join() {
        Ok(result) => result,
        Err(_) => {
            if let Ok(mut runtime) = state.runtime.lock() {
                let is_current_request = runtime
                    .connecting
                    .get(&device_id)
                    .is_some_and(|pending| pending.request_id == request_id);
                if is_current_request {
                    runtime.connecting.remove(&device_id);
                }
            }
            emit_snapshot(&app, state.inner().as_ref());
            return Err("接続スレッドが異常終了しました。".to_string());
        }
    };

    let (accepted_connection, failure) = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "状態のロックに失敗しました")?;
        let active = runtime
            .connecting
            .get(&device_id)
            .is_some_and(|pending| pending.request_id == request_id);
        if active {
            runtime.connecting.remove(&device_id);
        }
        match result {
            Ok(Some(connection)) if active => {
                // Minimal検証版と同じく、成功した接続のcloneをdevice ID別に保持する。
                runtime
                    .connections
                    .insert(device_id.clone(), connection.clone());
                (Some(connection), None)
            }
            Ok(Some(connection)) => {
                // 接続完了より先にキャンセルされた要求だけを閉じる。
                let _ = connection.Close();
                (None, None)
            }
            Ok(None) if active => (None, Some("接続を確立できませんでした。".to_string())),
            Err(error) if active => (None, Some(error)),
            Ok(None) | Err(_) => (None, None),
        }
    };

    if accepted_connection.is_some() {
        if let Err(error) = config::update(|saved| {
            saved.last_device_id = Some(device_id.clone());
            saved.last_device_name = Some(name.clone());
            Ok(())
        }) {
            config::append_log(&format!("最終接続端末を保存できません: {error}"));
        }
        config::append_log(&format!("接続しました: {name}"));
        let display_name = configured_device_name(&config::load(), &device_id, &name);
        emit_connection_event(&app, device_id, display_name, "connected");
    } else if let Some(message) = failure.as_ref() {
        config::append_log(&format!("接続エラー: {name}: {message}"));
        let _ = app.emit(
            "connection-progress",
            ProgressEvent {
                device_id,
                message: message.clone(),
            },
        );
    }
    emit_snapshot(&app, state.inner().as_ref());
    failure.map_or(Ok(()), Err)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    device_id: String,
    message: String,
}

#[tauri::command]
fn disconnect_device(
    app: AppHandle,
    device_id: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let (pending, connection) = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "状態のロックに失敗しました")?;
        cancel_disconnect_timer(&mut runtime, &device_id);
        (
            runtime.connecting.remove(&device_id),
            runtime.connections.remove(&device_id),
        )
    };
    if let Some(pending) = pending {
        pending.cancel.store(true, Ordering::Release);
    }
    // Minimal検証版で実機確認済みの要点: HashMapから対象IDだけをremoveし、
    // 取り出したAudioPlaybackConnectionだけをこのTauriコマンドでCloseする。
    let close_result = connection.map_or(Ok(()), |connection| {
        connection
            .Close()
            .map_err(|error| format!("対象端末のCloseに失敗しました: {error:?}"))
    });
    config::append_log(&format!("手動切断/キャンセル: {device_id}"));
    emit_snapshot(&app, state.inner().as_ref());
    close_result
}

fn disconnect_timer_is_current(state: &AppState, device_id: &str, timer_id: u64) -> bool {
    state.runtime.lock().ok().is_some_and(|runtime| {
        runtime
            .disconnect_timers
            .get(device_id)
            .is_some_and(|timer| timer.timer_id == timer_id)
            && runtime.connections.contains_key(device_id)
    })
}

fn disconnect_device_for_timer(app: &AppHandle, state: &AppState, device_id: &str, timer_id: u64) {
    let claimed = {
        let Ok(mut runtime) = state.runtime.lock() else {
            config::append_log("切断タイマー満了時に状態のロックに失敗しました。");
            return;
        };
        let is_current = runtime
            .disconnect_timers
            .get(device_id)
            .is_some_and(|timer| timer.timer_id == timer_id);
        if !is_current {
            None
        } else {
            runtime.disconnect_timers.remove(device_id);
            let name = runtime
                .devices
                .iter()
                .find(|device| device.id == device_id)
                .map(|device| device.name.clone())
                .unwrap_or_else(|| device_id.to_string());
            runtime
                .connections
                .remove(device_id)
                .map(|connection| (name, connection))
        }
    };

    let Some((name, connection)) = claimed else {
        return;
    };
    match connection.Close() {
        Ok(()) => {
            config::append_log(&format!("切断タイマーにより切断しました: {name}"));
            let display_name = configured_device_name(&config::load(), device_id, &name);
            emit_connection_event(app, device_id.to_string(), display_name, "disconnected");
        }
        Err(error) => config::append_log(&format!(
            "切断タイマーによる対象端末のCloseに失敗しました: {error:?}"
        )),
    }
    emit_snapshot(app, state);
}

#[tauri::command]
fn set_disconnect_timer(
    app: AppHandle,
    device_id: String,
    minutes: u64,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if minutes != 0 && !DISCONNECT_TIMER_MINUTES.contains(&minutes) {
        return Err("対応していないタイマー時間です。".to_string());
    }

    if minutes == 0 {
        let removed = {
            let mut runtime = state
                .runtime
                .lock()
                .map_err(|_| "状態のロックに失敗しました")?;
            cancel_disconnect_timer(&mut runtime, &device_id)
        };
        if removed {
            config::append_log("切断タイマーを解除しました。");
            emit_snapshot(&app, state.inner().as_ref());
        }
        return Ok(());
    }

    let duration = Duration::from_secs(minutes * 60);
    let deadline_epoch_seconds = SystemTime::now()
        .checked_add(duration)
        .and_then(|deadline| deadline.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .ok_or_else(|| "切断タイマーの期限を計算できませんでした。".to_string())?;
    let (name, timer_id, cancel) = {
        let mut runtime = state
            .runtime
            .lock()
            .map_err(|_| "状態のロックに失敗しました")?;
        if !runtime.connections.contains_key(&device_id) {
            return Err("接続中の端末にだけ切断タイマーを設定できます。".to_string());
        }
        let name = runtime
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .map(|device| device.name.clone())
            .ok_or_else(|| "デバイスが一覧にありません。再検索してください。".to_string())?;
        cancel_disconnect_timer(&mut runtime, &device_id);
        runtime.next_timer_id += 1;
        let timer_id = runtime.next_timer_id;
        let cancel = Arc::new(AtomicBool::new(false));
        runtime.disconnect_timers.insert(
            device_id.clone(),
            DisconnectTimer {
                timer_id,
                cancel: Arc::clone(&cancel),
                deadline_epoch_seconds,
            },
        );
        (name, timer_id, cancel)
    };

    config::append_log(&format!("切断タイマーを設定しました: {name} ({minutes}分)"));
    let display_name = configured_device_name(&config::load(), &device_id, &name);
    emit_snapshot(&app, state.inner().as_ref());

    let timer_app = app.clone();
    let timer_state = Arc::clone(state.inner());
    thread::spawn(move || {
        let warning_delay = duration.saturating_sub(Duration::from_secs(60));
        if !wait_for_timer(warning_delay, &cancel)
            || !disconnect_timer_is_current(&timer_state, &device_id, timer_id)
        {
            return;
        }

        let language = match config::load().display_language {
            Some(config::DisplayLanguage::Japanese) => i18n::Language::Japanese,
            Some(config::DisplayLanguage::English) => i18n::Language::English,
            None => i18n::system_language(),
        };
        let body = match language {
            i18n::Language::Japanese => format!("{display_name} を1分後に切断します。"),
            i18n::Language::English => {
                format!("{display_name} will be disconnected in 1 minute.")
            }
        };
        if let Err(error) = timer_app
            .notification()
            .builder()
            .title("Phone Audio Receiver")
            .body(body)
            .show()
        {
            config::append_log(&format!("切断タイマーの事前通知に失敗しました: {error}"));
        }

        if wait_for_timer(Duration::from_secs(60), &cancel) {
            disconnect_device_for_timer(&timer_app, timer_state.as_ref(), &device_id, timer_id);
        }
    });
    Ok(())
}

#[tauri::command]
fn set_device_auto_connect(
    app: AppHandle,
    device_id: String,
    enabled: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let known_device = state
        .runtime
        .lock()
        .map_err(|_| "状態のロックに失敗しました")?
        .devices
        .iter()
        .any(|device| device.id == device_id);
    if !known_device {
        return Err("デバイスが一覧にありません。再検索してください。".to_string());
    }

    config::update(|saved| {
        saved.auto_connect_device_ids.retain(|id| id != &device_id);
        if enabled {
            saved.auto_connect_device_ids.push(device_id.clone());
        }
        Ok(())
    })?;
    config::append_log(&format!(
        "端末ごとの自動接続設定を変更しました: {device_id} ({enabled})"
    ));
    emit_snapshot(&app, state.inner().as_ref());
    Ok(())
}

fn ensure_known_device(state: &AppState, device_id: &str) -> Result<(), String> {
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| "状態のロックに失敗しました")?;
    if runtime.devices.iter().any(|device| device.id == device_id) {
        Ok(())
    } else {
        Err("デバイスが一覧にありません。再検索してください。".to_string())
    }
}

#[tauri::command]
fn set_device_alias(
    app: AppHandle,
    device_id: String,
    alias: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    ensure_known_device(state.inner().as_ref(), &device_id)?;
    let alias = alias.trim().to_string();
    if alias.chars().count() > 80 || alias.chars().any(char::is_control) {
        return Err("表示名は制御文字を含まない80文字以内で入力してください。".to_string());
    }
    config::update(|saved| {
        let preference = saved
            .device_preferences
            .entry(device_id.clone())
            .or_default();
        preference.alias = alias.clone();
        Ok(())
    })?;
    config::append_log("端末の表示名設定を変更しました。");
    emit_snapshot(&app, state.inner().as_ref());
    Ok(())
}

#[tauri::command]
fn set_device_favorite(
    app: AppHandle,
    device_id: String,
    enabled: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    ensure_known_device(state.inner().as_ref(), &device_id)?;
    config::update(|saved| {
        saved
            .device_preferences
            .entry(device_id.clone())
            .or_default()
            .favorite = enabled;
        Ok(())
    })?;
    emit_snapshot(&app, state.inner().as_ref());
    Ok(())
}

#[tauri::command]
fn set_device_tray_pinned(
    app: AppHandle,
    device_id: String,
    enabled: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    ensure_known_device(state.inner().as_ref(), &device_id)?;
    config::update(|saved| {
        saved
            .device_preferences
            .entry(device_id.clone())
            .or_default()
            .tray_pinned = enabled;
        Ok(())
    })?;
    emit_snapshot(&app, state.inner().as_ref());
    Ok(())
}

#[tauri::command]
fn move_device(
    app: AppHandle,
    device_id: String,
    direction: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if direction != "up" && direction != "down" {
        return Err("対応していない移動方向です。".to_string());
    }
    ensure_known_device(state.inner().as_ref(), &device_id)?;

    let mut visible = snapshot(state.inner().as_ref()).devices;
    sort_devices_by_name(&mut visible);
    let mut ids: Vec<String> = visible.into_iter().map(|device| device.id).collect();
    if !move_device_id(&mut ids, &device_id, &direction) {
        return Ok(());
    }
    config::update(|saved| {
        saved.device_order = ids.clone();
        Ok(())
    })?;
    emit_snapshot(&app, state.inner().as_ref());
    Ok(())
}

#[tauri::command]
fn save_settings(state: State<'_, Arc<AppState>>, settings: Settings) -> Result<Settings, String> {
    let saved = config::update(|saved| {
        if saved.launch_at_login != settings.launch_at_login
            || (settings.launch_at_login && saved.start_minimized != settings.start_minimized)
        {
            set_startup_registration(settings.launch_at_login, settings.start_minimized)?;
        }
        saved.minimize_to_tray = settings.minimize_to_tray;
        saved.auto_connect = settings.auto_connect;
        saved.connection_notifications = settings.connection_notifications;
        saved.start_minimized = settings.start_minimized;
        saved.launch_at_login = settings.launch_at_login;
        saved.theme = match settings.theme.as_str() {
            "light" => config::ThemePreference::Light,
            "dark" => config::ThemePreference::Dark,
            _ => config::ThemePreference::System,
        };
        Ok(())
    })?;
    state
        .minimize_to_tray
        .store(saved.minimize_to_tray, Ordering::Release);
    Ok(saved.into())
}

#[tauri::command]
fn set_display_language(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    language: String,
) -> Result<(), String> {
    let display_language = match language.as_str() {
        "ja" => config::DisplayLanguage::Japanese,
        "en" => config::DisplayLanguage::English,
        _ => return Err("Unsupported display language".to_string()),
    };
    config::update(|saved| {
        saved.display_language = Some(display_language);
        Ok(())
    })?;
    emit_snapshot(&app, state.inner().as_ref());
    Ok(())
}

#[tauri::command]
fn diagnostics() -> String {
    #[cfg(debug_assertions)]
    {
        format!(
            "Version: {}\nBuild: {}\nArchitecture: {}\nClose strategy = {}\nExecutable: {}\nLog: {}\nData folder: {}",
            env!("CARGO_PKG_VERSION"),
            build_profile(),
            std::env::consts::ARCH,
            CLOSE_STRATEGY,
            executable_for_diagnostics(),
            config::log_path().display(),
            config::app_data_dir().display()
        )
    }
    #[cfg(not(debug_assertions))]
    {
        format!("Version: {}", env!("CARGO_PKG_VERSION"))
    }
}

#[tauri::command]
fn open_log_folder() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(config::app_data_dir())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn open_with_explorer(target: &str, description: &str) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("{description}を開けませんでした: {error}"))
}

#[tauri::command]
fn open_bluetooth_settings() -> Result<(), String> {
    open_with_explorer("ms-settings:bluetooth", "Bluetooth設定")
}

#[tauri::command]
fn open_sound_settings() -> Result<(), String> {
    open_with_explorer("ms-settings:sound", "サウンド設定")
}

#[tauri::command]
fn open_release_page() -> Result<(), String> {
    open_with_explorer(
        "https://github.com/mouiron/phone-audio-receiver/releases/latest",
        "GitHub Releases",
    )
}

#[tauri::command]
fn recent_log(state: State<'_, Arc<AppState>>) -> String {
    let mut devices: Vec<(String, String)> = state
        .runtime
        .lock()
        .map(|runtime| {
            runtime
                .devices
                .iter()
                .map(|device| (device.id.clone(), device.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let saved = config::load();
    if let (Some(id), Some(name)) = (saved.last_device_id, saved.last_device_name) {
        if !devices.iter().any(|(known_id, _)| known_id == &id) {
            devices.push((id, name));
        }
    }
    config::anonymize_log(&config::recent_log(30), &devices)
}

#[tauri::command]
fn hide_quick_window(app: AppHandle) {
    remember_window_mode(
        app.state::<Arc<AppState>>().inner().as_ref(),
        WindowMode::Quick,
    );
    if let Some(window) = app.get_webview_window("quick") {
        hide_webview_window(&window, "クイック操作画面");
    }
}

#[tauri::command]
fn switch_to_main_window(app: AppHandle) {
    show_main_window(&app);
}

#[tauri::command]
fn switch_to_quick_window(app: AppHandle) {
    show_quick_window(&app);
}

fn set_startup_registration(enabled: bool, start_minimized: bool) -> Result<(), String> {
    let mut key = HKEY::default();
    let create_result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            STARTUP_RUN_KEY,
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if !create_result.is_ok() {
        return Err(format!("レジストリキーを開けません ({create_result:?})"));
    }
    let result = if enabled {
        let exe = std::env::current_exe()
            .map_err(|error| format!("実行ファイルの場所を取得できません ({error})"))?;
        let mut command = format!("\"{}\"", exe.display());
        if start_minimized {
            command.push_str(" --start-minimized");
        }
        let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        let bytes = unsafe {
            std::slice::from_raw_parts(
                wide.as_ptr().cast::<u8>(),
                wide.len() * std::mem::size_of::<u16>(),
            )
        };
        unsafe { RegSetValueExW(key, STARTUP_VALUE_NAME, None, REG_SZ, Some(bytes)) }
    } else {
        unsafe { RegDeleteValueW(key, STARTUP_VALUE_NAME) }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if !result.is_ok() && (enabled || result != ERROR_FILE_NOT_FOUND) {
        return Err(format!(
            "自動起動を{}できません ({result:?})",
            if enabled { "登録" } else { "解除" }
        ));
    }
    Ok(())
}

/// 旧アプリ名で作られた自動起動値を取り除く。名称変更後に旧exeへの参照が残り、
/// Windowsログオン時に「ファイルが見つからない」状態になるのを防ぐ。
fn remove_legacy_startup_registration() {
    let mut key = HKEY::default();
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            STARTUP_RUN_KEY,
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if result.is_ok() {
        unsafe {
            let _ = RegDeleteValueW(key, LEGACY_STARTUP_VALUE_NAME);
            let _ = RegCloseKey(key);
        }
    }
}

/// ポータブル版をインターネットから取得した場合だけ、初回起動を許可した直後に
/// 同梱BATで実行ファイルのMark of the Webを解除する。インストーラー版にはBATを
/// 同梱しないため何もしない。差し替えられた外部BATを実行しないよう内容も照合する。
#[cfg(not(debug_assertions))]
fn run_portable_unblock_helper() {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = executable.parent() else {
        return;
    };
    let script = directory.join(PORTABLE_UNBLOCK_SCRIPT_NAME);
    let Ok(script_bytes) = std::fs::read(&script) else {
        return;
    };
    if script_bytes.as_slice() != PORTABLE_UNBLOCK_SCRIPT {
        return;
    }

    let mut zone_identifier = OsString::from(executable.as_os_str());
    zone_identifier.push(":Zone.Identifier");
    if std::fs::File::open(&zone_identifier).is_err() {
        return;
    }

    let command_line = format!("\"{}\"", script.display());
    let _ = Command::new("cmd.exe")
        .args(["/D", "/S", "/C"])
        .arg(command_line)
        .env("PHONE_AUDIO_RECEIVER_EXE", executable)
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

pub fn run() {
    #[cfg(not(debug_assertions))]
    run_portable_unblock_helper();

    #[cfg(debug_assertions)]
    {
        let executable = executable_for_diagnostics();
        // 開発版では、実際にどのバイナリと接続管理方式が動いたかを判別できるようにする。
        config::append_log(&format!(
            "起動要求: v{} / build={} / close-strategy={} / exe={executable}",
            env!("CARGO_PKG_VERSION"),
            build_profile(),
            CLOSE_STRATEGY
        ));
        eprintln!(
            "Phone Audio Receiver dev: log={} / exe={executable}",
            config::log_path().display()
        );
    }
    #[cfg(not(debug_assertions))]
    config::append_log(&format!("起動要求: v{}", env!("CARGO_PKG_VERSION")));

    if !acquire_single_instance() {
        #[cfg(debug_assertions)]
        config::append_log(&format!(
            "同じ{}版が既に起動しているため、新しい起動要求を既存ウィンドウへ転送しました。",
            build_profile()
        ));
        #[cfg(not(debug_assertions))]
        config::append_log("アプリが既に起動しているため、新しい起動要求を終了しました。");
        return;
    }
    remove_legacy_startup_registration();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(Arc::new(AppState::default()))
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                config::append_log(&format!(
                    "アプリ起動完了: v{} / build={} / close-strategy={} / exe={}",
                    env!("CARGO_PKG_VERSION"),
                    build_profile(),
                    CLOSE_STRATEGY,
                    executable_for_diagnostics()
                ));
                config::append_log(
                    "Tauriプロセス内の端末別接続管理を初期化しました（状態監視なし）。",
                );
            }
            let quick_window = WebviewWindowBuilder::new(
                app,
                "quick",
                WebviewUrl::App("index.html?mode=quick".into()),
            )
            .title(QUICK_WINDOW_TITLE)
            .inner_size(480.0, 620.0)
            .resizable(false)
            .skip_taskbar(true)
            .visible(false)
            .center()
            .build()?;
            hide_webview_window(&quick_window, "起動時のクイック操作画面");

            let menu = Menu::new(app)?;
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("アプリケーションアイコンを読み込めません")?;
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .tooltip("Phone Audio Receiver")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| handle_tray_menu_event(app, event.id().as_ref()))
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            ..
                        }
                    ) {
                        handle_tray_left_click(tray.app_handle());
                    }
                })
                .build(app)?;

            let state = app.state::<Arc<AppState>>();
            let value = snapshot(state.inner().as_ref());
            update_tray_menu(app.handle(), state.inner().as_ref(), &value)?;

            if started_minimized() && config::load().start_minimized {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "quick" {
                    remember_window_mode(
                        window.state::<Arc<AppState>>().inner().as_ref(),
                        WindowMode::Quick,
                    );
                    api.prevent_close();
                    let _ = window.hide();
                    return;
                }
                let minimize_to_tray = window
                    .state::<Arc<AppState>>()
                    .minimize_to_tray
                    .load(Ordering::Acquire);
                config::append_log(&format!(
                    "閉じるボタンを受け取りました (タスクトレイへ格納: {minimize_to_tray})"
                ));
                if minimize_to_tray {
                    remember_window_mode(
                        window.state::<Arc<AppState>>().inner().as_ref(),
                        WindowMode::Main,
                    );
                    api.prevent_close();
                    hide_window_for_tray(window);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_snapshot,
            refresh_devices,
            connect_device,
            disconnect_device,
            set_disconnect_timer,
            set_device_auto_connect,
            set_device_alias,
            set_device_favorite,
            set_device_tray_pinned,
            move_device,
            save_settings,
            set_display_language,
            diagnostics,
            open_log_folder,
            open_bluetooth_settings,
            open_sound_settings,
            open_release_page,
            recent_log,
            hide_quick_window,
            switch_to_main_window,
            switch_to_quick_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}

/// Tauri版だけを対象にした多重起動防止。Native UI版と同じアプリ名でも
/// 干渉しないよう、固有のミューテックス名とウィンドウクラスを使う。
fn acquire_single_instance() -> bool {
    let Ok(mutex) = (unsafe { CreateMutexW(None, false, SINGLE_INSTANCE_MUTEX) }) else {
        // 取得に失敗した場合でも起動不能にはせず、通常どおり起動する。
        return true;
    };
    let already_running =
        unsafe { windows::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        unsafe {
            let main_window = FindWindowW(TAURI_WINDOW_CLASS, PCWSTR::null())
                .or_else(|_| FindWindowW(PCWSTR::null(), MAIN_WINDOW_TITLE_WIDE));
            if let Ok(window) = main_window {
                // 通常画面を確実に取得できた場合だけクイック画面を隠す。
                // 取得失敗時に表示中の画面まで失われることを防ぐ。
                if let Ok(quick) = FindWindowW(PCWSTR::null(), QUICK_WINDOW_TITLE_WIDE) {
                    let _ = ShowWindow(quick, SW_HIDE);
                }
                let _ = ShowWindow(window, SW_RESTORE);
                let _ = ShowWindow(window, SW_SHOW);
                let _ = SetForegroundWindow(window);
            }
            let _ = windows::Win32::Foundation::CloseHandle(mutex);
        }
        return false;
    }
    // HANDLEはDropで閉じないため、プロセス終了時にOSが自動解放するまで有効。
    let _ = mutex;
    true
}

fn show_main_window(app: &AppHandle) {
    remember_window_mode(
        app.state::<Arc<AppState>>().inner().as_ref(),
        WindowMode::Main,
    );
    if let Some(quick) = app.get_webview_window("quick") {
        hide_webview_window(&quick, "クイック操作画面");
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn show_quick_window(app: &AppHandle) {
    remember_window_mode(
        app.state::<Arc<AppState>>().inner().as_ref(),
        WindowMode::Quick,
    );
    if let Some(main) = app.get_webview_window("main") {
        hide_webview_window(&main, "通常画面");
    }
    if let Some(window) = app.get_webview_window("quick") {
        let _ = window.center();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn handle_tray_left_click(app: &AppHandle) {
    let main_visible = app
        .get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    if main_visible {
        show_main_window(app);
        return;
    }

    if let Some(quick) = app.get_webview_window("quick") {
        if quick.is_visible().unwrap_or(false) {
            // Windowsから同じクリックに対する通知が重複しても閉じない。
            // 表示済みなら前面化だけを行い、操作を冪等にする。
            let _ = quick.set_focus();
            return;
        }
    }
    let last_mode = app
        .state::<Arc<AppState>>()
        .last_window_mode
        .lock()
        .map(|mode| *mode)
        .unwrap_or(WindowMode::Main);
    match last_mode {
        WindowMode::Main => show_main_window(app),
        WindowMode::Quick => show_quick_window(app),
    }
}

fn remember_window_mode(state: &AppState, mode: WindowMode) {
    if let Ok(mut last_mode) = state.last_window_mode.lock() {
        *last_mode = mode;
    }
}

fn handle_tray_menu_event(app: &AppHandle, id: &str) {
    match id {
        "quick" | "more" => show_quick_window(app),
        "show" => show_main_window(app),
        "quit" => app.exit(0),
        "refresh" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<Arc<AppState>>();
                if let Err(error) = refresh_devices(app.clone(), state) {
                    config::append_log(&format!("タスクトレイから再検索できませんでした: {error}"));
                }
            });
        }
        _ => {
            let action = app
                .state::<Arc<AppState>>()
                .tray_actions
                .lock()
                .ok()
                .and_then(|actions| actions.get(id).cloned());
            match action {
                Some(TrayAction::Connect(device_id)) => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<Arc<AppState>>();
                        if let Err(error) = connect_device(app.clone(), device_id, state) {
                            config::append_log(&format!(
                                "タスクトレイから接続できませんでした: {error}"
                            ));
                        }
                    });
                }
                Some(TrayAction::Disconnect(device_id)) => {
                    let state = app.state::<Arc<AppState>>();
                    if let Err(error) = disconnect_device(app.clone(), device_id, state) {
                        config::append_log(&format!(
                            "タスクトレイから切断できませんでした: {error}"
                        ));
                    }
                }
                None => {}
            }
        }
    }
}

fn started_minimized() -> bool {
    std::env::args()
        .skip(1)
        .any(|arg| arg == "--start-minimized")
}
