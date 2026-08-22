$ErrorActionPreference = 'Stop'

# 配布用exe、README、依存ライセンス、zipを一貫して作成する。
# 依存関係の安全性確認と完成exe監査は build_release.ps1 に委譲する。

$projectRoot = Split-Path -Parent $PSScriptRoot
$tauriConfig = Get-Content -Raw -LiteralPath (Join-Path $projectRoot 'src-tauri\tauri.conf.json') | ConvertFrom-Json
$version = $tauriConfig.version
$distributionBase = Join-Path $projectRoot 'distribution'
$distributionRoot = Join-Path $distributionBase "Phone Audio Receiver $version (Windows x64)"
$archivePath = Join-Path $distributionBase "Phone Audio Receiver $version (Windows x64).zip"
$stagingRoot = Join-Path $distributionBase ('.staging-' + [guid]::NewGuid().ToString('N'))
$releaseExe = Join-Path $projectRoot 'target\release\Phone Audio Receiver.exe'
$templatePath = Join-Path $PSScriptRoot 'distribution_README.template.md'

& (Join-Path $PSScriptRoot 'build_release.ps1')
if (-not (Test-Path -LiteralPath $releaseExe)) {
    throw "Release executable not found: $releaseExe"
}

New-Item -ItemType Directory -Path $distributionBase -Force | Out-Null
New-Item -ItemType Directory -Path $stagingRoot | Out-Null
try {
    $stagingExe = Join-Path $stagingRoot 'Phone Audio Receiver.exe'
    Copy-Item -LiteralPath $releaseExe -Destination $stagingExe
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $stagingExe).Hash
    $readme = [System.IO.File]::ReadAllText($templatePath)
    $readme = $readme.Replace('{{VERSION}}', $version).Replace('{{SHA256}}', $hash)
    [System.IO.File]::WriteAllText((Join-Path $stagingRoot 'README.md'), $readme, [System.Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $stagingRoot 'LICENSE')

    & (Join-Path $PSScriptRoot 'generate_third_party_licenses.ps1') -DistributionRoot $stagingRoot

    $expectedFiles = @(
        'Phone Audio Receiver.exe'
        'README.md'
        'LICENSE'
        'THIRD_PARTY_LICENSES.txt'
        'THIRD_PARTY_NOTICES.md'
    ) | Sort-Object
    $actualFiles = @(Get-ChildItem -LiteralPath $stagingRoot -File | ForEach-Object Name | Sort-Object)
    $unexpected = Compare-Object -ReferenceObject $expectedFiles -DifferenceObject $actualFiles
    if ($unexpected) {
        throw 'Distribution contents differ from the expected five-file allowlist.'
    }

    if (Test-Path -LiteralPath $distributionRoot) {
        $basePath = [System.IO.Path]::GetFullPath($distributionBase).TrimEnd('\') + '\'
        $targetPath = [System.IO.Path]::GetFullPath($distributionRoot)
        if (-not $targetPath.StartsWith($basePath, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to replace a directory outside the distribution folder: $targetPath"
        }
        Remove-Item -LiteralPath $targetPath -Recurse -Force
    }
    Move-Item -LiteralPath $stagingRoot -Destination $distributionRoot

    Compress-Archive -LiteralPath $distributionRoot -DestinationPath $archivePath -Force
    Write-Host "Distribution directory created: $distributionRoot"
    Write-Host "Distribution archive created: $archivePath"
    Write-Host "SHA-256: $hash"
}
finally {
    if (Test-Path -LiteralPath $stagingRoot) {
        Remove-Item -LiteralPath $stagingRoot -Recurse -Force
    }
}
