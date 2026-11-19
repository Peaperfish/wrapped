# Run with PowerShell from any directory. Produces a portable per-user folder.
param([switch]$FrontendBuilt)
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '../..')
try {
    if (-not $FrontendBuilt) {
        Push-Location dashboard/ui
        try {
            npm ci --no-audit --no-fund
            if ($LASTEXITCODE) { throw "npm ci failed" }
            npm run build
            if ($LASTEXITCODE) { throw "frontend build failed" }
        } finally { Pop-Location }
    }
    cargo build -q --config profile.release.strip=true --locked --release --workspace --bins --features 'wrapped-dashboard/custom-protocol,wrapped-collector/windows-launcher'
    if ($LASTEXITCODE) { throw "release build failed" }
    $target = (cargo metadata -q --no-deps --format-version 1 | ConvertFrom-Json).target_directory
    $bundle = Join-Path $target 'windows-package'
    New-Item -ItemType Directory -Force $bundle | Out-Null
    foreach ($name in @('Wrapped.exe', 'wrapped-collector.exe', 'wrapped-dashboard.exe')) {
        Copy-Item (Join-Path $target "release/$name") (Join-Path $bundle $name) -Force
    }
    $loader = Join-Path $target 'release/WebView2Loader.dll'
    if (Test-Path $loader) { Copy-Item $loader $bundle -Force }
    Get-ChildItem $bundle -File | Select-Object Name, Length
} finally { Pop-Location }
