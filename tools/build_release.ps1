$ErrorActionPreference = 'Stop'

# Tauri本稼働版の単体配布用exeを作る。
# このファイルがある tauri ディレクトリから実行する。
$env:CARGO_TARGET_DIR = Join-Path (Get-Location) 'target'

& (Join-Path $PSScriptRoot 'security_check.ps1')
npm run frontend:build
cargo auditable build --locked --manifest-path src-tauri\Cargo.toml --release --bin bluetooth_phone_audio_receiver_tauri --features tauri/custom-protocol

$source = Resolve-Path target\release\bluetooth_phone_audio_receiver_tauri.exe
$destination = Join-Path (Split-Path $source) 'Phone Audio Receiver.exe'
Copy-Item -LiteralPath $source -Destination $destination -Force
& (Join-Path $PSScriptRoot 'audit_release.ps1')

Write-Host 'Tauri release executable created:' $destination
