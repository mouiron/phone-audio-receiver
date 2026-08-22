# Privacy Policy

最終更新日: 2026-08-22

Phone Audio Receiverは、テレメトリ、利用状況、広告識別子、連絡先、音声内容を収集または外部送信しません。アプリは受信した音声をWindowsの標準機能を通じて既定の再生デバイスへ出力し、音声内容を保存しません。

## ローカルに保存する情報

設定と診断ログは、利用者のWindows環境にある次のフォルダーへ保存されます。

```text
%APPDATA%\phone_audio_receiver\
```

保存内容には、アプリ設定、接続操作、Bluetooth端末名、BluetoothデバイスID、アプリやデータフォルダーのパスが含まれる場合があります。これらの情報はアプリから自動送信されません。

アプリ内のログ表示と**匿名化済みログをコピー**では、端末名、完全なBluetoothデバイスID、ユーザー名、ユーザーパス、実行ファイルの絶対パスを匿名化します。同じ端末の対応関係は匿名ラベルで維持されます。

生の`log.txt`と`log.previous.txt`には個人の環境を推測できる情報が残る場合があります。公開Issue、チャット、第三者へ共有しないでください。

## 削除

アプリを終了した後、上記フォルダーを削除すると、保存された設定とログを削除できます。

## Privacy Policy (English summary)

Phone Audio Receiver does not collect or transmit telemetry, usage data, advertising identifiers, contacts, or audio content. Settings and diagnostic logs are stored locally under `%APPDATA%\phone_audio_receiver\`. Raw logs may contain device names, complete Bluetooth device IDs, user paths, and executable paths. Do not share raw logs publicly; use **Copy anonymized log** in the application instead.
