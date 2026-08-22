param(
    [string]$DistributionRoot
)

$ErrorActionPreference = 'Stop'

# Windows x64 の配布exeに関係するRust依存と、フロントエンドに同梱するnpm依存の
# ライセンス一覧・本文を生成する。ネットワークへ接続せず、ロックファイルとローカル
# Cargo/npmキャッシュだけを参照するため、配布前に再現可能。

$projectRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($DistributionRoot)) {
    $tauriConfig = Get-Content -Raw -LiteralPath (Join-Path $projectRoot 'src-tauri\tauri.conf.json') | ConvertFrom-Json
    $DistributionRoot = Join-Path $projectRoot "distribution\Phone Audio Receiver $($tauriConfig.version) (Windows x64)"
}
$noticePath = Join-Path $distributionRoot 'THIRD_PARTY_NOTICES.md'
$textsPath = Join-Path $distributionRoot 'THIRD_PARTY_LICENSES.txt'

if (-not (Test-Path -LiteralPath $distributionRoot)) {
    throw "Distribution directory not found: $distributionRoot"
}

$metadataJson = cargo metadata --offline --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $projectRoot 'src-tauri\Cargo.toml')
if ($LASTEXITCODE -ne 0) {
    throw 'Could not read locked Rust dependency metadata.'
}
$metadata = $metadataJson | ConvertFrom-Json
$externalRustPackages = $metadata.packages | Where-Object {
    -not $_.manifest_path.StartsWith($projectRoot, [System.StringComparison]::OrdinalIgnoreCase)
}

$licenseNamePattern = '^(LICENSE|LICENCE|COPYING|NOTICE)([._-].*)?$'
$licenseTexts = @{}
$rows = [System.Collections.Generic.List[object]]::new()
$missingLicenseFiles = [System.Collections.Generic.List[string]]::new()

function Add-LicenseTexts {
    param(
        [string]$PackageName,
        [string]$PackageVersion,
        [string]$Directory
    )

    $files = Get-ChildItem -LiteralPath $Directory -File | Where-Object { $_.Name -match $licenseNamePattern }
    if (-not $files) {
        $script:missingLicenseFiles.Add("$PackageName $PackageVersion")
        return
    }
    foreach ($file in $files) {
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash
        if (-not $script:licenseTexts.ContainsKey($hash)) {
            $script:licenseTexts[$hash] = [PSCustomObject]@{
                Source = "$PackageName $PackageVersion ($($file.Name))"
                Text = [System.IO.File]::ReadAllText($file.FullName)
            }
        }
    }
}

foreach ($package in $externalRustPackages | Sort-Object name, version) {
    $rows.Add([PSCustomObject]@{
        Ecosystem = 'Rust'
        Package = $package.name
        Version = $package.version
        License = if ([string]::IsNullOrWhiteSpace($package.license)) { 'Not specified' } else { $package.license }
    })
    Add-LicenseTexts -PackageName $package.name -PackageVersion $package.version -Directory (Split-Path -Parent $package.manifest_path)
}

$lock = Get-Content -Raw -LiteralPath (Join-Path $projectRoot 'package-lock.json') | ConvertFrom-Json -AsHashtable
foreach ($entry in $lock.packages.GetEnumerator() | Where-Object { $_.Key -ne '' -and -not $_.Value.dev } | Sort-Object Key) {
    $packagePath = Join-Path $projectRoot $entry.Key
    $rows.Add([PSCustomObject]@{
        Ecosystem = 'npm'
        Package = ($entry.Key -replace '^node_modules/', '')
        Version = $entry.Value.version
        License = if ([string]::IsNullOrWhiteSpace($entry.Value.license)) { 'Not specified' } else { $entry.Value.license }
    })
    Add-LicenseTexts -PackageName ($entry.Key -replace '^node_modules/', '') -PackageVersion $entry.Value.version -Directory $packagePath
}

$notice = [System.Text.StringBuilder]::new()
[void]$notice.AppendLine('# Third-Party Notices')
[void]$notice.AppendLine()
[void]$notice.AppendLine('This distribution includes the following locked dependencies used for the Windows x64 release.')
[void]$notice.AppendLine('License texts and notices collected from the locally installed package sources are in `THIRD_PARTY_LICENSES.txt`.')
[void]$notice.AppendLine()
[void]$notice.AppendLine("Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss K')")
[void]$notice.AppendLine()
[void]$notice.AppendLine('## Dependency list')
[void]$notice.AppendLine()
[void]$notice.AppendLine('| Ecosystem | Package | Version | Declared license |')
[void]$notice.AppendLine('| --- | --- | --- | --- |')
foreach ($row in $rows | Sort-Object Ecosystem, Package, Version) {
    [void]$notice.AppendLine("| $($row.Ecosystem) | $($row.Package) | $($row.Version) | $($row.License) |")
}
if ($missingLicenseFiles.Count -gt 0) {
    [void]$notice.AppendLine()
    [void]$notice.AppendLine('## Packages without a root license file')
    [void]$notice.AppendLine()
    [void]$notice.AppendLine('These packages declare a license in package metadata but do not include a root-level license file in the locally installed source. Their declared license terms are represented by the collected common license texts where available.')
    foreach ($package in $missingLicenseFiles | Sort-Object) {
        [void]$notice.AppendLine("- $package")
    }
}
[System.IO.File]::WriteAllText($noticePath, $notice.ToString(), [System.Text.UTF8Encoding]::new($false))

$texts = [System.Text.StringBuilder]::new()
[void]$texts.AppendLine('THIRD-PARTY LICENSE TEXTS AND NOTICES')
[void]$texts.AppendLine('=====================================')
[void]$texts.AppendLine()
[void]$texts.AppendLine('Each section is a unique license or notice file collected from the locked dependency sources listed in THIRD_PARTY_NOTICES.md.')
foreach ($entry in $licenseTexts.Values | Sort-Object Source) {
    [void]$texts.AppendLine()
    [void]$texts.AppendLine(('=' * 80))
    [void]$texts.AppendLine($entry.Source)
    [void]$texts.AppendLine(('=' * 80))
    [void]$texts.AppendLine($entry.Text.TrimEnd())
    [void]$texts.AppendLine()
}
[System.IO.File]::WriteAllText($textsPath, $texts.ToString(), [System.Text.UTF8Encoding]::new($false))

Write-Host "Generated $noticePath"
Write-Host "Generated $textsPath"
Write-Host "Dependencies: $($rows.Count); unique license/notice texts: $($licenseTexts.Count); packages without root license files: $($missingLicenseFiles.Count)"
