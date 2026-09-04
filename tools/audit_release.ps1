param(
    [string]$BinaryPath
)

$ErrorActionPreference = 'Stop'

# cargo-auditableでビルドしたWindows exeに埋め込まれた依存情報を監査する。
# Cargo.lockの条件付きLinux依存ではなく、配布物に実際に入った依存関係だけが対象。

$projectRoot = Split-Path $PSScriptRoot -Parent
$binaryPath = if ($BinaryPath) {
    [System.IO.Path]::GetFullPath($BinaryPath)
}
else {
    Join-Path $projectRoot 'target\release\Phone Audio Receiver.exe'
}
if (-not (Test-Path -LiteralPath $binaryPath)) {
    throw "配布用exeが見つかりません: $binaryPath"
}

# cargo-auditは正常動作時にもデータベース取得状況をstderrへ出力する。
# Windows PowerShellのネイティブstderr例外化を避けながら終了コードを厳格に判定するため、
# Start-Processでstdout、stderr、終了コードを個別に取得する。
$temporaryRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$captureId = [guid]::NewGuid().ToString('N')
$standardOutputPath = Join-Path $temporaryRoot "phone-audio-receiver-audit-$captureId.out"
$standardErrorPath = Join-Path $temporaryRoot "phone-audio-receiver-audit-$captureId.err"
try {
    $auditProcess = Start-Process -FilePath 'cargo.exe' `
        -ArgumentList ('audit bin "' + $binaryPath + '"') `
        -RedirectStandardOutput $standardOutputPath `
        -RedirectStandardError $standardErrorPath `
        -WindowStyle Hidden -Wait -PassThru
    $auditExitCode = $auditProcess.ExitCode
    $auditOutput =
        [System.IO.File]::ReadAllText($standardErrorPath) +
        [System.IO.File]::ReadAllText($standardOutputPath)
}
finally {
    Remove-Item -LiteralPath $standardOutputPath -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $standardErrorPath -Force -ErrorAction SilentlyContinue
}
Write-Host $auditOutput
if ($auditExitCode -ne 0) {
    throw 'The release executable failed the RustSec audit.'
}

$baselinePath = Join-Path $PSScriptRoot 'rustsec-release-warning-baseline.txt'
$knownWarningIds = Get-Content $baselinePath |
    Where-Object { $_ -and -not $_.StartsWith('#') } |
    Sort-Object -Unique
$currentWarningIds = @(
    [regex]::Matches($auditOutput, 'RUSTSEC-\d{4}-\d{4}') |
        ForEach-Object { $_.Value } |
        Sort-Object -Unique
)
$warningDifference = @(
    Compare-Object -ReferenceObject @($knownWarningIds) -DifferenceObject @($currentWarningIds)
)
$unexpectedWarnings = @(
    $warningDifference |
        Where-Object SideIndicator -eq '=>' |
        ForEach-Object { $_.InputObject }
)
$missingWarnings = @(
    $warningDifference |
        Where-Object SideIndicator -eq '<=' |
        ForEach-Object { $_.InputObject }
)
if ($unexpectedWarnings -or $missingWarnings) {
    $details = @()
    if ($unexpectedWarnings) { $details += 'new: ' + ($unexpectedWarnings -join ', ') }
    if ($missingWarnings) { $details += 'no longer present: ' + ($missingWarnings -join ', ') }
    throw ('Release executable RustSec warning baseline changed (' + ($details -join '; ') + '). Review and update the baseline deliberately.')
}

Write-Host ''
Write-Host ('Release executable audit completed successfully ({0} reviewed informational warnings).' -f $currentWarningIds.Count) -ForegroundColor Green
