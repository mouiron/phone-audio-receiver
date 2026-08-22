# Contributing

IssueやPull Requestを歓迎します。Bluetooth接続処理はWindowsと実機の挙動に強く依存するため、次の方針を守ってください。

## プライバシー

- 実際のスマートフォン名は使わず、`端末A`、`端末B`などに置き換えてください。
- 完全なBluetoothデバイスID、Windowsユーザー名、ユーザーフォルダー、実行ファイルの絶対パスを投稿しないでください。
- 生の`log.txt`と`log.previous.txt`を共有しないでください。
- ログはアプリの**匿名化済みログをコピー**から取得してください。

## 接続管理の不変条件

- `AudioPlaybackConnection`はTauriプロセス内で端末ID別に保持します。
- 接続はWinRTを初期化した専用MTAスレッドで`TryCreateFromId -> Start -> Open`の順に行います。
- 切断時は対象端末のIDだけを接続表から取り出して`Close`します。
- 1台を切断したとき、接続中の別端末を閉じたり、作り直したり、再接続したりしてはいけません。
- 音声制御用の子プロセス、常駐コントローラー、独自JSON IPCを導入しません。
- `AudioPlaybackConnection::State`を外部切断の定期監視に使用しません。
- 外部切断後の自動再接続を追加しません。
- 起動時に選択端末へ接続する機能は維持します。

## 自動検証

フロントエンド変更：

```powershell
npm run frontend:build
```

Rust変更：

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
```

依存関係の追加・更新または配布前：

```powershell
.\tools\security_check.ps1
```

## Bluetooth接続変更時の手動回帰確認

最低限、次の両方を実機で確認してください。

1. 端末Aを接続 → 端末Bを接続 → 端末Bを切断 → 端末Aを切断
2. 端末Bを接続 → 端末Aを接続 → 端末Aを切断 → 端末Bを切断

各手順では、切断対象ではない端末の接続表示と音声が維持されることを確認します。接続処理を変更したPull Requestでは、Windowsバージョン、確認したアプリバージョン、接続・切断順、結果を記載してください。
