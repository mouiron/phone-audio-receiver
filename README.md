# Phone Audio Receiver

スマートフォンのBluetooth音声をWindows PCで受信し、Windowsの既定の再生デバイスから再生するアプリです。

Phone Audio Receiver turns a Windows PC into a Bluetooth audio receiver for paired smartphones. The application UI supports Japanese and English.

> 現在はベータ版です。2台のスマートフォンによる同時接続と個別切断を実機確認済みです。アプリ内に台数制限はありませんが、3台以上の同時接続は未検証で、Bluetoothアダプターやドライバーに依存します。

## ダウンロード

最新版は[GitHub Releases](https://github.com/mouiron/phone-audio-receiver/releases/latest)からダウンロードできます。

zipを展開し、`Phone Audio Receiver.exe`を実行してください。現在の配布バイナリはコード署名されていないため、初回実行時にMicrosoft Defender SmartScreenの警告が表示される場合があります。ダウンロード元とReleaseに記載されたSHA-256を確認し、信頼できる場合にだけ実行してください。

## 主な機能

- ペアリング済みスマートフォンのBluetooth音声をPCで再生
- 複数端末の個別接続・個別切断
- タスクトレイへの格納と復帰
- 選択した端末への起動時自動接続
- Windowsログオン時の自動起動
- Windowsの優先表示言語に応じた日本語・英語表示
- 問題報告に使用できる匿名化済みログ

音声はWindows標準の`AudioPlaybackConnection`を使用し、Windowsの既定の再生デバイスへ出力します。仮想オーディオドライバーや音声制御用の子プロセスは使用しません。

## 動作環境

- 64bit版Windows 10 バージョン2004以降、またはWindows 11
- Bluetooth機能
- 音声再生に対応した、Windowsとペアリング済みのスマートフォン
- Microsoft Edge WebView2 Runtime

通常のWindows 10/11にはWebView2 Runtimeが導入されています。画面が表示されない場合は、[Microsoft公式サイト](https://developer.microsoft.com/microsoft-edge/webview2/)から導入してください。

## 使い方

1. WindowsのBluetooth設定でスマートフォンをペアリングします。
2. Phone Audio Receiverを起動します。
3. **再検索**を押します。
4. 表示された端末の**接続**を押します。
5. スマートフォンで音声を再生します。

既定では、ウィンドウを閉じてもアプリは終了せず、タスクトレイへ格納されます。完全に終了する場合は、タスクトレイアイコンを右クリックして**終了**を選択してください。

## 既知の制約

- 出力先はWindowsの既定の再生デバイスです。アプリから出力先を個別指定できません。
- 端末ごとの音量・ミュート操作はできません。Windowsの音量ミキサーまたはスマートフォン側で調整してください。
- スマートフォン側やWindowsのBluetooth側から切断した場合、画面が「接続中」のままになることがあります。対象端末の**切断**を押して管理状態を解除し、改めて**接続**してください。
- 外部切断後の自動再接続は、Windows APIによる状態誤判定を避けるため意図的に提供していません。起動時の自動接続は別機能です。
- Bluetoothの同時接続可能台数と安定性は、Bluetoothアダプター、ドライバー、Windows、端末側の実装に依存します。

詳細は[KNOWN_ISSUES.md](KNOWN_ISSUES.md)を参照してください。

## プライバシーと問題報告

アプリはテレメトリを収集せず、アプリから外部へログを送信しません。ログはローカルに保存されます。生ログには端末名、完全なBluetoothデバイスID、ユーザーフォルダーが含まれる場合があるため、公開Issueへ添付しないでください。

問題報告には、アプリの**匿名化済みログをコピー**で取得した内容を使用してください。詳細は[PRIVACY.md](PRIVACY.md)を参照してください。

## 開発

必要な環境：

- Windows
- Node.js 22以降
- Rust stable（MSVCツールチェーン）
- Microsoft Edge WebView2 Runtime

```powershell
npm install
npm run tauri:dev
```

検証：

```powershell
npm run frontend:build
cargo fmt --all -- --check
cargo test --workspace --locked
```

依存関係を追加・更新した場合と配布前には、次を実行します。

```powershell
.\tools\security_check.ps1
```

開発上の不変条件と手動回帰確認は[CONTRIBUTING.md](CONTRIBUTING.md)を参照してください。

## 構成

- `web/` — Tauriフロントエンド
- `src-tauri/` — TauriバックエンドとWindows統合
- `core/` — Bluetooth接続、設定、国際化のRustロジック
- `tools/` — セキュリティ確認、releaseビルド、配布物作成

## ライセンス

このプロジェクトは[MIT License](LICENSE)で公開されています。配布バイナリが利用するサードパーティーソフトウェアのライセンスは、Releaseの配布物に含まれる`THIRD_PARTY_NOTICES.md`と`THIRD_PARTY_LICENSES.txt`を参照してください。
