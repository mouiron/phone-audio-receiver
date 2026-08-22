$ErrorActionPreference = 'Stop'

# Tauri本稼働版の単体配布用exeを作る。
# このファイルがある tauri ディレクトリから実行する。
$projectRoot = Split-Path $PSScriptRoot -Parent
$env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target'

# Rustのpanic位置などにビルドPCのユーザーパスやワークスペース絶対パスを
# 埋め込まない。CARGO_ENCODED_RUSTFLAGSは区切り文字形式なので、パスに空白が
# 含まれる環境でも各remap指定を1引数としてrustcへ渡せる。
$previousEncodedRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
$encodedSeparator = [char]0x1f
$encodedRustFlags = @()
if ($previousEncodedRustFlags) {
    $encodedRustFlags += $previousEncodedRustFlags -split $encodedSeparator
}
if ($env:USERPROFILE) {
    $encodedRustFlags += "--remap-path-prefix=$env:USERPROFILE=%USERPROFILE%"
}
$encodedRustFlags += "--remap-path-prefix=$projectRoot=."
$env:CARGO_ENCODED_RUSTFLAGS = $encodedRustFlags -join $encodedSeparator

try {
    & (Join-Path $PSScriptRoot 'security_check.ps1')
    npm run frontend:build
    cargo auditable build --locked --manifest-path src-tauri\Cargo.toml --release --bin bluetooth_phone_audio_receiver_tauri --features tauri/custom-protocol

    $source = Resolve-Path (Join-Path $env:CARGO_TARGET_DIR 'release\bluetooth_phone_audio_receiver_tauri.exe')
    $destination = Join-Path (Split-Path $source) 'Phone Audio Receiver.exe'
    Copy-Item -LiteralPath $source -Destination $destination -Force
    & (Join-Path $PSScriptRoot 'audit_release.ps1')

    Write-Host 'Tauri release executable created:' $destination
}
finally {
    if ($null -eq $previousEncodedRustFlags) {
        Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
    }
    else {
        $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedRustFlags
    }
}
