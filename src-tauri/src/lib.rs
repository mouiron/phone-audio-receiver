use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use bluetooth_phone_audio_receiver_core::{bt, config, i18n};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};
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
    next_request_id: u64,
}

struct PendingConnection {
    request_id: u64,
    cancel: Arc<AtomicBool>,
}

struct AppState {
    runtime: Mutex<Runtime>,
    /// ×ボタンの処理中に設定ファイルを読み直さず、直近に確定した設定を使う。
    /// ウィンドウイベントとファイルI/Oが競合する余地をなくす。
    minimize_to_tray: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(Runtime::default()),
            minimize_to_tray: AtomicBool::new(config::load().minimize_to_tray),
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView {
    id: String,
    name: String,
    status: String,
    auto_connect: bool,
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

fn snapshot(state: &AppState) -> AppSnapshot {
    let saved = config::load();
    let runtime = state.runtime.lock().expect("runtime lock poisoned");
    let devices = runtime
        .devices
        .iter()
        .map(|device| DeviceView {
            id: device.id.clone(),
            name: device.name.clone(),
            status: if runtime.connections.contains_key(&device.id) {
                "connected"
            } else if runtime.connecting.contains_key(&device.id) {
                "connecting"
            } else {
                "disconnected"
            }
            .to_string(),
            auto_connect: saved.auto_connect_device_ids.contains(&device.id),
        })
        .collect();
    AppSnapshot {
        devices,
        last_device_id: saved.last_device_id.clone(),
        auto_connect_device_ids: saved.auto_connect_device_ids.clone(),
        display_language: match i18n::system_language() {
            i18n::Language::Japanese => "ja",
            i18n::Language::English => "en",
        }
        .to_string(),
        settings: saved.into(),
    }
}

fn emit_snapshot(app: &AppHandle, state: &AppState) {
    let _ = app.emit("app-state-changed", snapshot(state));
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
        emit_connection_event(&app, device_id, name, "connected");
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

pub fn run() {
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
            let language = i18n::system_language();
            let show = MenuItem::with_id(
                app,
                "show",
                language.select("開く", "Open"),
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
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("アプリケーションアイコンを読み込めません")?;
            TrayIconBuilder::with_id("main")
                .icon(icon)
                .tooltip("Phone Audio Receiver")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            ..
                        } | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        }
                    ) {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            if started_minimized() && config::load().start_minimized {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray = window
                    .state::<Arc<AppState>>()
                    .minimize_to_tray
                    .load(Ordering::Acquire);
                config::append_log(&format!(
                    "閉じるボタンを受け取りました (タスクトレイへ格納: {minimize_to_tray})"
                ));
                if minimize_to_tray {
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
            set_device_auto_connect,
            save_settings,
            diagnostics,
            open_log_folder,
            recent_log
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
            if let Ok(window) = FindWindowW(TAURI_WINDOW_CLASS, PCWSTR::null()) {
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
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn started_minimized() -> bool {
    std::env::args()
        .skip(1)
        .any(|arg| arg == "--start-minimized")
}
