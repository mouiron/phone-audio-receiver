//! アプリ設定の永続化 (%APPDATA%\phone_audio_receiver\config.json) と
//! 簡易ログファイル出力 (%APPDATA%\phone_audio_receiver\log.txt)。
//!
//! GUIを閉じても次回起動時に「前回のデバイス」「トレイ格納するか」等を
//! 覚えておけるようにする。ファイル1個で完結する単純なJSON永続化なので、
//! DBやレジストリは使わない(SREの感覚で言えば「状態はファイルに素直に書く」方針)。

use serde::{Deserialize, Deserializer, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const APP_DATA_ID: &str = "phone_audio_receiver";
static CONFIG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

fn config_lock() -> &'static Mutex<()> {
    CONFIG_LOCK.get_or_init(|| Mutex::new(()))
}

fn log_lock() -> &'static Mutex<()> {
    LOG_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    Light,
    System,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayLanguage {
    Japanese,
    English,
}

fn deserialize_display_language<'de, D>(
    deserializer: D,
) -> Result<Option<DisplayLanguage>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(match value.as_deref() {
        Some("ja" | "japanese") => Some(DisplayLanguage::Japanese),
        Some("en" | "english") => Some(DisplayLanguage::English),
        _ => None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// 最後に接続したデバイスのID (AudioPlaybackConnection用の device id 文字列)
    #[serde(default)]
    pub last_device_id: Option<String>,
    /// 最後に接続したデバイスの表示名 (UI初期表示用)
    #[serde(default)]
    pub last_device_name: Option<String>,
    /// ウィンドウを閉じたときにタスクトレイへ格納するか(false なら普通に終了)
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
    /// 起動時に前回のデバイスへ自動接続するか
    #[serde(default)]
    pub auto_connect: bool,
    /// 起動時の自動接続対象にするデバイスID。空の場合は後方互換として前回の端末を使う。
    #[serde(default)]
    pub auto_connect_device_ids: Vec<String>,
    /// 接続・切断の状態変化をWindows通知で知らせるか
    #[serde(default)]
    pub connection_notifications: bool,
    /// ユーザーログオン時にアプリを自動起動するか
    #[serde(default)]
    pub launch_at_login: bool,
    /// 自動起動時にメインウィンドウを表示せず、タスクトレイに格納するか
    #[serde(default = "default_true")]
    pub start_minimized: bool,
    /// 旧バージョンとの設定互換性のため保持する表示テーマ設定。
    /// GUIは現在ライトテーマに固定し、起動時にLightへ移行する。
    #[serde(default)]
    pub theme: ThemePreference,
    /// ユーザーが画面上で明示的に選択した表示言語。
    /// 未選択の既存設定ではWindowsの優先表示言語を使用する。
    #[serde(default, deserialize_with = "deserialize_display_language")]
    pub display_language: Option<DisplayLanguage>,
}

fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            last_device_id: None,
            last_device_name: None,
            minimize_to_tray: true,
            auto_connect: false,
            auto_connect_device_ids: Vec::new(),
            connection_notifications: false,
            launch_at_login: false,
            start_minimized: true,
            theme: ThemePreference::Light,
            display_language: None,
        }
    }
}

pub fn app_data_dir() -> PathBuf {
    app_data_dir_for(APP_DATA_ID)
}

/// アプリ識別子ごとの設定・ログ保存先を返す。
/// 本稼働のTauri版は従来の保存先を維持し、バックアップ版は別名を使う。
pub fn app_data_dir_for(application_id: &str) -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    let dir = PathBuf::from(base).join(application_id);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn config_path_for(application_id: &str) -> PathBuf {
    app_data_dir_for(application_id).join("config.json")
}

pub fn log_path() -> PathBuf {
    log_path_for(APP_DATA_ID)
}

pub fn log_path_for(application_id: &str) -> PathBuf {
    app_data_dir_for(application_id).join("log.txt")
}

/// 設定を読み込む。ファイルが無い/壊れている場合はデフォルト値を返す
/// (初回起動やファイル破損で落ちるのは避けたいので、失敗を握りつぶす設計)。
pub fn load() -> AppConfig {
    load_from(APP_DATA_ID)
}

pub fn load_from(application_id: &str) -> AppConfig {
    let _guard = config_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    load_from_unlocked(application_id)
}

fn load_from_unlocked(application_id: &str) -> AppConfig {
    match std::fs::read_to_string(config_path_for(application_id)) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

/// 設定を一時ファイルへ書いた後に置換し、途中終了によるJSON破損を防ぐ。
pub fn save(cfg: &AppConfig) -> Result<(), String> {
    save_to(APP_DATA_ID, cfg)
}

pub fn save_to(application_id: &str, cfg: &AppConfig) -> Result<(), String> {
    let _guard = config_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    save_to_unlocked(application_id, cfg)
}

/// load→変更→saveを1つのロック内で行い、接続ワーカーとUI操作の更新競合を防ぐ。
pub fn update(
    change: impl FnOnce(&mut AppConfig) -> Result<(), String>,
) -> Result<AppConfig, String> {
    let _guard = config_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut cfg = load_from_unlocked(APP_DATA_ID);
    change(&mut cfg)?;
    save_to_unlocked(APP_DATA_ID, &cfg)?;
    Ok(cfg)
}

fn save_to_unlocked(application_id: &str, cfg: &AppConfig) -> Result<(), String> {
    let path = config_path_for(application_id);
    write_config_file(&path, cfg)
}

fn write_config_file(path: &Path, cfg: &AppConfig) -> Result<(), String> {
    let temporary_path = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(cfg)
        .map_err(|error| format!("設定をJSONへ変換できません: {error}"))?;
    std::fs::write(&temporary_path, text)
        .map_err(|error| format!("一時設定ファイルを書き込めません: {error}"))?;
    std::fs::rename(&temporary_path, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary_path);
        format!("設定ファイルを置換できません: {error}")
    })
}

/// ログファイルに1行追記する(タイムスタンプ付き)。
/// トラブルシュート用途(接続に失敗したとき、後から経緯を追えるようにする)。
pub fn append_log(line: &str) {
    append_log_to(APP_DATA_ID, line)
}

pub fn append_log_to(application_id: &str, line: &str) {
    let _guard = log_lock().lock().unwrap_or_else(|error| error.into_inner());
    let path = log_path_for(application_id);
    if std::fs::metadata(&path)
        .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
        .unwrap_or(false)
    {
        let rotated_path = path.with_file_name("log.previous.txt");
        let _ = std::fs::remove_file(&rotated_path);
        let _ = std::fs::rename(&path, rotated_path);
    }
    let now = unix_timestamp();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "[{now}] {line}");
    }
}

/// 画面表示用に、ログ末尾を最大 `max_lines` 行だけ返す。
///
/// ログが長期間蓄積しても、UIへ巨大な文字列を渡して操作不能にならないようにする。
/// 読み込みに失敗した場合は空文字列とし、ログ閲覧自体がアプリ動作を妨げないようにする。
pub fn recent_log(max_lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(log_path()) else {
        return String::new();
    };
    let mut lines: Vec<&str> = text.lines().rev().take(max_lines).collect();
    lines.reverse();
    lines.join("\n")
}

/// アプリ内表示・クリップボード共有用にログから個人情報を取り除く。
///
/// 生ログはローカルでの詳細調査用に変更せず保持し、UIへ渡す文字列だけを匿名化する。
/// 同じ端末の名前とIDには同じラベルを割り当てるため、時系列や端末間の関係は追跡できる。
pub fn anonymize_log(text: &str, devices: &[(String, String)]) -> String {
    let user_profile = std::env::var("USERPROFILE").ok();
    let app_data = std::env::var("APPDATA").ok();
    let executable = std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    anonymize_log_with_context(
        text,
        devices,
        user_profile.as_deref(),
        app_data.as_deref(),
        executable.as_deref(),
    )
}

#[derive(Debug)]
struct AnonymousDevice {
    id: String,
    names: Vec<String>,
}

fn anonymize_log_with_context(
    text: &str,
    devices: &[(String, String)],
    user_profile: Option<&str>,
    app_data: Option<&str>,
    executable: Option<&str>,
) -> String {
    let mut identities = Vec::<AnonymousDevice>::new();
    for (id, name) in devices {
        add_device_identity(&mut identities, id, Some(name.as_str()));
    }
    for (id, name) in bluetooth_identities_from_log(text) {
        add_device_identity(&mut identities, &id, name.as_deref());
    }

    let mut anonymized = text.to_string();
    for (index, identity) in identities.iter().enumerate() {
        let label = anonymous_device_label(index);
        anonymized =
            replace_ascii_case_insensitive(&anonymized, &identity.id, &format!("<{label}-ID>"));
    }

    let mut names: Vec<(String, String)> = identities
        .iter()
        .enumerate()
        .flat_map(|(index, identity)| {
            let replacement = format!("<{}>", anonymous_device_label(index));
            identity
                .names
                .iter()
                .cloned()
                .map(move |name| (name, replacement.clone()))
        })
        .collect();
    // 「Phone」と「My Phone」のような名前がある場合、長い名前を先に置換する。
    names.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));
    for (name, replacement) in names {
        anonymized = anonymized.replace(&name, &replacement);
    }

    if let Some(executable) = executable.filter(|value| !value.is_empty()) {
        anonymized = replace_path(&anonymized, executable, "<APP_EXECUTABLE>");
    }
    if let Some(app_data) = app_data.filter(|value| !value.is_empty()) {
        anonymized = replace_path(&anonymized, app_data, "%APPDATA%");
    }
    if let Some(profile) = user_profile.filter(|value| !value.is_empty()) {
        anonymized = replace_path(&anonymized, profile, "%USERPROFILE%");
        let forward_slash_profile = profile.replace('\\', "/");
        if forward_slash_profile != profile {
            anonymized = replace_path(&anonymized, &forward_slash_profile, "%USERPROFILE%");
        }
    }
    anonymized
}

fn add_device_identity(identities: &mut Vec<AnonymousDevice>, id: &str, name: Option<&str>) {
    if id.is_empty() {
        return;
    }
    if let Some(existing) = identities
        .iter_mut()
        .find(|identity| identity.id.eq_ignore_ascii_case(id))
    {
        if let Some(name) = name.filter(|value| !value.is_empty()) {
            if !existing.names.iter().any(|known| known == name) {
                existing.names.push(name.to_string());
            }
        }
        return;
    }
    identities.push(AnonymousDevice {
        id: id.to_string(),
        names: name
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .into_iter()
            .collect(),
    });
}

fn bluetooth_identities_from_log(text: &str) -> Vec<(String, Option<String>)> {
    const MARKER: &str = "bthenum#";
    const DEVICE_PREFIX: &str = r"\\?\";

    let lowercase = text.to_ascii_lowercase();
    let mut cursor = 0;
    let mut result = Vec::new();
    while let Some(relative) = lowercase[cursor..].find(MARKER) {
        let marker_start = cursor + relative;
        let start = marker_start
            .checked_sub(DEVICE_PREFIX.len())
            .filter(|candidate| &text[*candidate..marker_start] == DEVICE_PREFIX)
            .unwrap_or(marker_start);
        let end = text[start..]
            .char_indices()
            .find(|(_, character)| character.is_whitespace() || *character == ')')
            .map(|(offset, _)| start + offset)
            .unwrap_or(text.len());
        let id = text[start..end].to_string();

        let line_start = text[..start].rfind('\n').map_or(0, |index| index + 1);
        let prefix = &text[line_start..start];
        let name = prefix
            .strip_suffix(" (")
            .and_then(|value| value.rsplit_once(": ").map(|(_, name)| name))
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        result.push((id, name));
        cursor = end.max(marker_start + MARKER.len());
    }
    result
}

fn anonymous_device_label(index: usize) -> String {
    if index < 26 {
        format!("端末{}", char::from(b'A' + index as u8))
    } else {
        format!("端末{}", index + 1)
    }
}

fn replace_path(input: &str, path: &str, replacement: &str) -> String {
    if path.is_ascii() {
        replace_ascii_case_insensitive(input, path, replacement)
    } else {
        input.replace(path, replacement)
    }
}

fn replace_ascii_case_insensitive(input: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || !needle.is_ascii() {
        return input.replace(needle, replacement);
    }
    let needle_lowercase = needle.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    loop {
        let remainder = &input[cursor..];
        let lowercase = remainder.to_ascii_lowercase();
        let Some(relative) = lowercase.find(&needle_lowercase) else {
            output.push_str(remainder);
            break;
        };
        let start = cursor + relative;
        output.push_str(&input[cursor..start]);
        output.push_str(replacement);
        cursor = start + needle.len();
    }
    output
}

/// 依存クレートを増やさないための簡易タイムスタンプ(UNIX epoch秒)。
/// 人間が読みやすい日時ではないが、ログの前後関係を追う分には十分。
fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_config_save_can_replace_an_existing_file() {
        let path = std::env::temp_dir().join(format!(
            "phone_audio_receiver_config_test_{}.json",
            std::process::id()
        ));
        let temporary_path = path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&temporary_path);

        let first = AppConfig {
            auto_connect: false,
            ..AppConfig::default()
        };
        write_config_file(&path, &first).expect("first save should succeed");

        let second = AppConfig {
            auto_connect: true,
            ..AppConfig::default()
        };
        write_config_file(&path, &second).expect("replacement save should succeed");

        let saved: AppConfig = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("saved config should be readable"),
        )
        .expect("saved config should contain valid JSON");
        assert!(saved.auto_connect);

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(temporary_path);
    }

    #[test]
    fn log_anonymization_replaces_device_identity_and_user_profile() {
        let phone_a_id = r"\\?\BTHENUM#{service}_VID&0001#DEVICE_A\SNK";
        let phone_b_id = r"\\?\BTHENUM#{service}_VID&0002#DEVICE_B\SNK";
        let log = format!(
            "[1] 接続試行開始: Personal Phone ({phone_a_id})\n\
             [2] 接続しました: Personal Phone\n\
             [3] 接続試行開始: Work Phone ({phone_b_id})\n\
             [4] 手動切断/キャンセル: {phone_a_id}\n\
             [5] exe=C:\\Users\\alice\\AppData\\Local\\app.exe\n\
             [6] data=C:\\Users\\alice\\AppData\\Roaming\\phone_audio_receiver\n\
             [7] export=C:\\Users\\alice\\Documents\\log.txt"
        );
        let devices = vec![
            (phone_a_id.to_string(), "Personal Phone".to_string()),
            (phone_b_id.to_string(), "Work Phone".to_string()),
        ];

        let anonymized = anonymize_log_with_context(
            &log,
            &devices,
            Some(r"C:\Users\alice"),
            Some(r"C:\Users\alice\AppData\Roaming"),
            Some(r"C:\Users\alice\AppData\Local\app.exe"),
        );

        assert!(!anonymized.contains("Personal Phone"));
        assert!(!anonymized.contains("Work Phone"));
        assert!(!anonymized.contains(phone_a_id));
        assert!(!anonymized.contains(phone_b_id));
        assert!(!anonymized.contains("alice"));
        assert!(anonymized.contains("<端末A>"));
        assert!(anonymized.contains("<端末A-ID>"));
        assert!(anonymized.contains("<端末B>"));
        assert!(anonymized.contains("<端末B-ID>"));
        assert!(anonymized.contains("exe=<APP_EXECUTABLE>"));
        assert!(anonymized.contains("data=%APPDATA%\\phone_audio_receiver"));
        assert!(anonymized.contains("export=%USERPROFILE%\\Documents\\log.txt"));
    }

    #[test]
    fn log_anonymization_discovers_an_unlisted_device_from_connection_log() {
        let device_id = r"\\?\BTHENUM#{service}_VID&0001#UNKNOWN\SNK";
        let log = format!("[1] 接続試行開始: Private Name ({device_id})");

        let anonymized = anonymize_log_with_context(&log, &[], None, None, None);

        assert!(!anonymized.contains("Private Name"));
        assert!(!anonymized.contains(device_id));
        assert_eq!(anonymized, "[1] 接続試行開始: <端末A> (<端末A-ID>)");
    }

    #[test]
    fn legacy_reconnect_setting_is_ignored_and_not_saved_again() {
        let config: AppConfig =
            serde_json::from_str(r#"{"reconnect_on_disconnect":true,"auto_connect":true}"#)
                .expect("legacy config should remain readable");

        assert!(config.auto_connect);
        let serialized = serde_json::to_string(&config).expect("config should serialize");
        assert!(!serialized.contains("reconnect_on_disconnect"));
    }

    #[test]
    fn display_language_is_optional_and_round_trips() {
        let legacy: AppConfig = serde_json::from_str(r#"{"auto_connect":true}"#)
            .expect("config without a language should remain readable");
        assert_eq!(legacy.display_language, None);

        let config = AppConfig {
            display_language: Some(DisplayLanguage::English),
            ..AppConfig::default()
        };
        let serialized = serde_json::to_string(&config).expect("config should serialize");
        let restored: AppConfig =
            serde_json::from_str(&serialized).expect("saved config should remain readable");
        assert_eq!(restored.display_language, Some(DisplayLanguage::English));

        let invalid: AppConfig =
            serde_json::from_str(r#"{"auto_connect":true,"display_language":"invalid"}"#)
                .expect("an unsupported language should not invalidate other settings");
        assert!(invalid.auto_connect);
        assert_eq!(invalid.display_language, None);
    }
}
