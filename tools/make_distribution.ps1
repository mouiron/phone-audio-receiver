$ErrorActionPreference = 'Stop'

# 配布用exe、README、依存ライセンスを含むNSISインストーラーを一貫して作成する。
# 依存関係の安全性確認と完成exe監査は build_release.ps1 に委譲する。

$projectRoot = Split-Path -Parent $PSScriptRoot
$tauriConfig = Get-Content -Raw -LiteralPath (Join-Path $projectRoot 'src-tauri\tauri.conf.json') | ConvertFrom-Json
$version = $tauriConfig.version
$distributionBase = Join-Path $projectRoot 'distribution'
$installerName = "Phone Audio Receiver $version (Windows x64)-setup.exe"
$installerPath = Join-Path $distributionBase $installerName
$portableName = "Phone Audio Receiver $version Portable (Windows x64)"
$portableRoot = Join-Path $distributionBase $portableName
$portableArchiveName = "$portableName.zip"
$portableArchivePath = Join-Path $distributionBase $portableArchiveName
$checksumPath = Join-Path $distributionBase 'SHA256SUMS.txt'
$stagingRoot = Join-Path $distributionBase ('.installer-resources-' + [guid]::NewGuid().ToString('N'))
$portableStagingRoot = Join-Path $distributionBase ('.portable-' + [guid]::NewGuid().ToString('N'))
$bundleConfigPath = Join-Path $distributionBase ('.installer-config-' + [guid]::NewGuid().ToString('N') + '.json')
$releaseExe = Join-Path $projectRoot 'target\release\Phone Audio Receiver.exe'
$bundledExe = Join-Path $projectRoot 'target\release\bluetooth_phone_audio_receiver_tauri.exe'
$portableUnblockScript = Join-Path $projectRoot 'src-tauri\resources\unblock_downloaded_app.bat'
$bundleDirectory = Join-Path $projectRoot 'target\release\bundle\nsis'
$tauriCli = Join-Path $projectRoot 'node_modules\.bin\tauri.cmd'
$templatePath = Join-Path $PSScriptRoot 'distribution_README.template.md'
$portableTemplatePath = Join-Path $PSScriptRoot 'portable_README.template.md'

& (Join-Path $PSScriptRoot 'build_release.ps1')
if (-not (Test-Path -LiteralPath $releaseExe)) {
    throw "Release executable not found: $releaseExe"
}
if (-not (Test-Path -LiteralPath $tauriCli)) {
    throw "Tauri CLI not found: $tauriCli"
}

New-Item -ItemType Directory -Path $distributionBase -Force | Out-Null
New-Item -ItemType Directory -Path $stagingRoot | Out-Null
New-Item -ItemType Directory -Path $portableStagingRoot | Out-Null
try {
    $readme = [System.IO.File]::ReadAllText($templatePath)
    $readme = $readme.Replace('{{VERSION}}', $version)
    [System.IO.File]::WriteAllText((Join-Path $stagingRoot 'README.md'), $readme, [System.Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $stagingRoot 'LICENSE')

    & (Join-Path $PSScriptRoot 'generate_third_party_licenses.ps1') -DistributionRoot $stagingRoot

    $expectedFiles = @(
        'README.md'
        'LICENSE'
        'THIRD_PARTY_LICENSES.txt'
        'THIRD_PARTY_NOTICES.md'
    ) | Sort-Object
    $actualFiles = @(Get-ChildItem -LiteralPath $stagingRoot -File | ForEach-Object Name | Sort-Object)
    $unexpected = Compare-Object -ReferenceObject $expectedFiles -DifferenceObject $actualFiles
    if ($unexpected) {
        throw 'Installer resource contents differ from the expected four-file allowlist.'
    }

    $resources = [ordered]@{}
    foreach ($name in $expectedFiles) {
        $resources[(Join-Path $stagingRoot $name)] = $name
    }
    $bundleConfig = [ordered]@{
        bundle = [ordered]@{
            resources = $resources
        }
    }
    $bundleConfigJson = $bundleConfig | ConvertTo-Json -Depth 6
    [System.IO.File]::WriteAllText($bundleConfigPath, $bundleConfigJson, [System.Text.UTF8Encoding]::new($false))

    $bundleStartedAt = Get-Date
    & $tauriCli bundle --bundles nsis --config $bundleConfigPath --ci --no-sign
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri NSIS bundle failed with exit code $LASTEXITCODE."
    }

    $generatedInstallers = @(
        Get-ChildItem -LiteralPath $bundleDirectory -File -Filter '*-setup.exe' |
            Where-Object LastWriteTime -ge $bundleStartedAt.AddSeconds(-2) |
            Sort-Object LastWriteTime -Descending
    )
    if ($generatedInstallers.Count -ne 1) {
        throw "Expected one newly generated NSIS installer, found $($generatedInstallers.Count)."
    }

    # TauriはNSIS用メタデータをrelease exeへ追加してからインストーラーへ格納する。
    # その最終状態のexeにもcargo-auditable情報が残り、既知脆弱性がないことを確認する。
    & (Join-Path $PSScriptRoot 'audit_release.ps1') -BinaryPath $bundledExe

    Copy-Item -LiteralPath $generatedInstallers[0].FullName -Destination $installerPath -Force
    $installerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installerPath).Hash

    $portableExePath = Join-Path $portableStagingRoot 'Phone Audio Receiver.exe'
    Copy-Item -LiteralPath $releaseExe -Destination $portableExePath
    Copy-Item -LiteralPath $portableUnblockScript -Destination (Join-Path $portableStagingRoot 'unblock_downloaded_app.bat')
    foreach ($name in $expectedFiles | Where-Object { $_ -ne 'README.md' }) {
        Copy-Item -LiteralPath (Join-Path $stagingRoot $name) -Destination (Join-Path $portableStagingRoot $name)
    }
    $portableExeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $portableExePath).Hash
    $portableReadme = [System.IO.File]::ReadAllText($portableTemplatePath)
    $portableReadme = $portableReadme.Replace('{{VERSION}}', $version).Replace('{{EXE_SHA256}}', $portableExeHash)
    [System.IO.File]::WriteAllText((Join-Path $portableStagingRoot 'README.md'), $portableReadme, [System.Text.UTF8Encoding]::new($false))

    $expectedPortableFiles = @(
        'Phone Audio Receiver.exe'
        'unblock_downloaded_app.bat'
        'README.md'
        'LICENSE'
        'THIRD_PARTY_LICENSES.txt'
        'THIRD_PARTY_NOTICES.md'
    ) | Sort-Object
    $actualPortableFiles = @(Get-ChildItem -LiteralPath $portableStagingRoot -File | ForEach-Object Name | Sort-Object)
    $unexpectedPortableFiles = Compare-Object -ReferenceObject $expectedPortableFiles -DifferenceObject $actualPortableFiles
    if ($unexpectedPortableFiles) {
        throw 'Portable distribution contents differ from the expected six-file allowlist.'
    }

    if (Test-Path -LiteralPath $portableRoot) {
        $distributionBasePath = [System.IO.Path]::GetFullPath($distributionBase).TrimEnd('\') + '\'
        $portableTargetPath = [System.IO.Path]::GetFullPath($portableRoot)
        if (-not $portableTargetPath.StartsWith($distributionBasePath, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to replace a directory outside the distribution folder: $portableTargetPath"
        }
        Remove-Item -LiteralPath $portableTargetPath -Recurse -Force
    }
    Move-Item -LiteralPath $portableStagingRoot -Destination $portableRoot
    Compress-Archive -LiteralPath $portableRoot -DestinationPath $portableArchivePath -Force
    $portableArchiveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $portableArchivePath).Hash

    $checksumLines = @(
        "$installerHash *$installerName"
        "$portableArchiveHash *$portableArchiveName"
    ) -join "`r`n"
    [System.IO.File]::WriteAllText($checksumPath, "$checksumLines`r`n", [System.Text.UTF8Encoding]::new($false))

    Write-Host "Distribution installer created: $installerPath"
    Write-Host "Portable distribution archive created: $portableArchivePath"
    Write-Host "Checksum file created: $checksumPath"
    Write-Host "Installer SHA-256: $installerHash"
    Write-Host "Portable archive SHA-256: $portableArchiveHash"
}
finally {
    if (Test-Path -LiteralPath $stagingRoot) {
        Remove-Item -LiteralPath $stagingRoot -Recurse -Force
    }
    if (Test-Path -LiteralPath $bundleConfigPath) {
        Remove-Item -LiteralPath $bundleConfigPath -Force
    }
    if (Test-Path -LiteralPath $portableStagingRoot) {
        Remove-Item -LiteralPath $portableStagingRoot -Recurse -Force
    }
}
