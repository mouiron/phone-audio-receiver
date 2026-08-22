//! AudioPlaybackConnection (A2DP Sink) を扱うコアロジック。
//!
//! 設計方針: OSが持つ AudioPlaybackConnection の sink 機能を使い、
//! 呼び出し順序は Start() → Open() (公式ドキュメント通りの順序)。
//! Open() の Status が Success 以外の場合や、実際に State が
//! Opened(1) にならない場合は、接続オブジェクトを作り直して
//! リトライする(既知の不安定挙動への対処)。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::core::HSTRING;
use windows::Devices::Enumeration::DeviceInformation;
use windows::Foundation::TypedEventHandler;
use windows::Media::Audio::AudioPlaybackConnection;

/// 一覧表示用の簡易デバイス情報。
#[derive(Debug, Clone)]
pub struct DeviceEntry {
    pub id: String,
    pub name: String,
}

/// 音声受信(sink)に利用可能な、ペア済みBluetoothデバイスを列挙する。
pub fn enumerate_devices() -> windows::core::Result<Vec<DeviceEntry>> {
    let selector: HSTRING = AudioPlaybackConnection::GetDeviceSelector()?;
    let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;

    let count = devices.Size()?;
    let mut result = Vec::with_capacity(count as usize);
    for i in 0..count {
        let d = devices.GetAt(i)?;
        result.push(DeviceEntry {
            id: d.Id()?.to_string(),
            name: d.Name()?.to_string(),
        });
    }
    Ok(result)
}

/// device_id に対して接続を試みる。`on_log` には進捗メッセージが逐次渡される
/// (GUI版はこれをログウィンドウやファイルへ、CLI版はそのままprintlnへ流す)。
///
/// 戻り値: 成功時は `Some(接続済みAudioPlaybackConnection)`、
/// 全リトライ失敗時は `Ok(None)`。
pub fn connect_with_retry(
    device_id: &str,
    cancel: &AtomicBool,
    mut on_log: impl FnMut(String),
) -> windows::core::Result<Option<AudioPlaybackConnection>> {
    const MAX_ATTEMPTS: u32 = 8;
    const WAIT_TIMEOUT: Duration = Duration::from_secs(8);
    const RETRY_DELAY: Duration = Duration::from_secs(5);

    let id = HSTRING::from(device_id);
    let language = crate::i18n::system_language();
    let cancelled = || {
        language
            .select(
                "接続試行をキャンセルしました。",
                "The connection attempt was cancelled.",
            )
            .to_string()
    };

    for attempt in 1..=MAX_ATTEMPTS {
        if cancel.load(Ordering::Acquire) {
            on_log(cancelled());
            return Ok(None);
        }
        on_log(match language {
            crate::i18n::Language::Japanese => {
                format!("[試行 {attempt}/{MAX_ATTEMPTS}] 接続をオープンしています...")
            }
            crate::i18n::Language::English => {
                format!("[Attempt {attempt}/{MAX_ATTEMPTS}] Opening the connection...")
            }
        });

        // リトライのたびに新しいインスタンスを作る
        // (Close済みインスタンスの使い回しは ERROR_INVALID_STATE の原因になる)。
        let connection = AudioPlaybackConnection::TryCreateFromId(&id)?;

        // Native UI版と同じライフサイクルを維持する。イベントハンドラーが
        // connectionのcloneを保持し、状態変化時にその端末のStateだけを参照する。
        let connection_for_state = connection.clone();
        connection.StateChanged(&TypedEventHandler::new(move |_sender, _| {
            let _ = connection_for_state.State();
            Ok(())
        }))?;

        // 呼び出し順序は Start() → Open() が正しい
        // (Start=システムへの登録、Open=実際の接続確立)。
        connection.Start()?;

        if cancel.load(Ordering::Acquire) {
            let _ = connection.Close();
            on_log(cancelled());
            return Ok(None);
        }

        let open_result = connection.Open()?;
        let status = open_result.Status()?;
        on_log(format!(
            "  Open() Status = {} (0=Success/1=RequestTimedOut/2=DeniedBySystem/3=UnknownFailure)",
            status.0
        ));

        if status.0 != 0 {
            if let Ok(err) = open_result.ExtendedError() {
                on_log(format!("  ExtendedError = {err:?}"));
            }
            let _ = connection.Close();
            if attempt < MAX_ATTEMPTS && !wait_or_cancel(RETRY_DELAY, cancel) {
                on_log(cancelled());
                return Ok(None);
            }
            continue;
        }

        if wait_for_opened(&connection, WAIT_TIMEOUT, cancel)? {
            on_log(
                language
                    .select(
                        "接続が確立しました(State = Opened)。",
                        "The connection was established (State = Opened).",
                    )
                    .to_string(),
            );
            return Ok(Some(connection));
        }

        if cancel.load(Ordering::Acquire) {
            let _ = connection.Close();
            on_log(cancelled());
            return Ok(None);
        }

        on_log(match language {
            crate::i18n::Language::Japanese => format!(
                "  {WAIT_TIMEOUT:?} 待っても State が Opened になりませんでした。再試行します。"
            ),
            crate::i18n::Language::English => {
                format!("  State did not become Opened within {WAIT_TIMEOUT:?}. Retrying.")
            }
        });
        let _ = connection.Close();
        if attempt < MAX_ATTEMPTS && !wait_or_cancel(RETRY_DELAY, cancel) {
            on_log(cancelled());
            return Ok(None);
        }
    }

    on_log(match language {
        crate::i18n::Language::Japanese => {
            format!("{MAX_ATTEMPTS}回試行しましたが接続を確立できませんでした。")
        }
        crate::i18n::Language::English => {
            format!("Could not establish a connection after {MAX_ATTEMPTS} attempts.")
        }
    });
    Ok(None)
}

fn wait_for_opened(
    connection: &AudioPlaybackConnection,
    timeout: Duration,
    cancel: &AtomicBool,
) -> windows::core::Result<bool> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        if connection.State()?.0 == 1 {
            return Ok(true);
        }
        if !wait_or_cancel(Duration::from_millis(300), cancel) {
            return Ok(false);
        }
    }
    Ok(!cancel.load(Ordering::Acquire) && connection.State()?.0 == 1)
}

/// 待機中もキャンセルを素早く反映する。再試行間隔が長くても、UIからの
/// 「切断」操作が最大100ms程度でワーカースレッドに届くようにする。
fn wait_or_cancel(duration: Duration, cancel: &AtomicBool) -> bool {
    const POLL_INTERVAL: Duration = Duration::from_millis(100);
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        std::thread::sleep(remaining.min(POLL_INTERVAL));
    }
    !cancel.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn wait_or_cancel_returns_early_after_cancellation() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_from_worker = Arc::clone(&cancel);
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            cancel_from_worker.store(true, Ordering::Release);
        });

        let started = std::time::Instant::now();
        assert!(!wait_or_cancel(Duration::from_secs(1), &cancel));
        assert!(started.elapsed() < Duration::from_millis(250));
        worker.join().unwrap();
    }
}
