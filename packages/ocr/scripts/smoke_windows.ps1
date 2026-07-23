[CmdletBinding()]
param(
    [switch]$CheckZotero
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = (Resolve-Path (Join-Path $ProjectRoot "..\..")).Path
$Runner = Join-Path $PSScriptRoot "run_windows.ps1"
$Python = Join-Path $ProjectRoot ".venv\Scripts\python.exe"

& $Runner -Install worker --help | Out-Null
& $Runner paddleocr --help | Out-Null

& $Python -c "import requests, fitz, pypdf, markdown; print('python-deps-ok')"

$envPath = Join-Path $RepoRoot ".env"
if (Test-Path -LiteralPath $envPath) {
    $keys = Select-String -LiteralPath $envPath -Pattern '^[A-Za-z_][A-Za-z0-9_]*=' |
        ForEach-Object { ($_.Line -split '=', 2)[0] }
    Write-Host ("env-keys-present: " + ($keys -join ", "))
}
else {
    Write-Host "env-missing"
}

$storage = Join-Path $env:USERPROFILE "Zotero\storage"
if (Test-Path -LiteralPath $storage) {
    Write-Host "zotero-storage-ok"
}
else {
    Write-Host "zotero-storage-missing"
}

if ($CheckZotero) {
    & $Runner worker --dry-run --limit 1 --max-runtime-minutes 1
}
