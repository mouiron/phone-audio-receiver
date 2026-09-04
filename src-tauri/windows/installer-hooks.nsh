; アプリが設定したWindowsログオン時の自動起動を、アンインストール完了時に取り除く。
; 設定・ログなどのユーザーデータは、意図しない消失を避けるため削除しない。
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "PhoneAudioReceiver"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "BluetoothPhoneAudioReceiverTauri"
!macroend
