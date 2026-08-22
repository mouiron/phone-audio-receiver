$ErrorActionPreference = 'Stop'

# ビルド前に、ロックファイルの既知の脆弱性とnpm公開署名を確認する。
# Windows配布物に実際に含まれるRust依存は、ビルド後にaudit_release.ps1で監査する。

$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    npm audit
    npm audit signatures
    $auditReport = cargo audit --file Cargo.lock --json | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) {
        throw 'RustSec detected a known vulnerability or a yanked dependency.'
    }
    $warningCount = @(
        $auditReport.warnings.PSObject.Properties |
            ForEach-Object { @($_.Value) }
    ).Count
    Write-Host ('Cargo.lock: no vulnerabilities; {0} informational warnings (checked against the final exe after build).' -f $warningCount)
    Write-Host ''
    Write-Host 'Security checks completed: no blocking known vulnerabilities found.' -ForegroundColor Green
}
finally {
    Pop-Location
}
