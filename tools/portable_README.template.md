# Phone Audio Receiver Portable

スマートフォンのBluetooth音声をWindows PCで受信し、Windowsの既定の再生デバイスから再生するアプリです。

バージョン: {{VERSION}}

## 起動方法

1. zipを任意のローカルフォルダーへ展開します。
2. `Phone Audio Receiver.exe`を実行します。
3. 初回にMicrosoft Defender SmartScreenが表示された場合は、配布元とSHA-256を確認し、信頼できる場合にだけ **詳細情報** → **実行** を選びます。

初回起動を許可すると、アプリは同梱の`unblock_downloaded_app.bat`を非表示で一度だけ実行し、`Phone Audio Receiver.exe`のMark of the Webを解除します。BATは内容が配布時のものと完全に一致する場合にだけ実行され、フォルダー内のほかのファイルやWindows全体のSmartScreen設定は変更しません。

同じEXEの2回目以降の起動では、通常SmartScreenは表示されません。zipを再展開した場合や新しいバージョンへ置き換えた場合は、新しいEXEに対して初回確認が必要です。組織のセキュリティポリシーによって実行自体が禁止されている環境では利用できません。

アプリ本体のSHA-256:

```text
{{EXE_SHA256}}
```

## アプリの終了と削除

ウィンドウを閉じても、既定ではタスクトレイへ格納されます。完全に終了する場合は、タスクトレイのPhone Audio Receiverを右クリックして **終了** を選びます。

ポータブル版を削除する場合は、完全終了後に展開したフォルダーを削除します。設定とログは次のフォルダーに保存されているため、再利用しない場合は別途手動で削除します。

```text
%APPDATA%\phone_audio_receiver\
```

生ログには端末名、完全なBluetooth ID、ユーザーフォルダーが含まれる場合があるため、削除前に第三者へ共有しないでください。

## 同梱文書

- `README.md` — この説明書
- `LICENSE` — アプリ本体のMITライセンス
- `THIRD_PARTY_NOTICES.md` — 利用している依存ライブラリの一覧と宣言ライセンス
- `THIRD_PARTY_LICENSES.txt` — 依存ライブラリのライセンス本文・NOTICE
