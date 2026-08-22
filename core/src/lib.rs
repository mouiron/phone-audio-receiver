//! bluetooth_phone_audio_receiver 共有ライブラリ
//!
//! CLI版 (`src/main.rs`) と GUI版 (`src/bin/gui.rs`) の両方から
//! 同じ Bluetooth 接続ロジック・設定永続化ロジックを使い回すためのモジュール。
//! ロジックを1箇所にまとめることで、片方だけ直して動作がずれる事故を防ぐ。

pub mod bt;
pub mod config;
pub mod i18n;
