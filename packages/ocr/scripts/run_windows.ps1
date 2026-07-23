[CmdletBinding()]
param(
    [ValidateSet("worker", "mineru", "html", "inventory", "normalize", "paddleocr")]
    [string]$Task = "worker",

    [switch]$Install,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ScriptArgs
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$RepoRoot = (Resolve-Path (Join-Path $ProjectRoot "..\..")).Path
$VenvPython = Join-Path $ProjectRoot ".venv\Scripts\python.exe"
$VenvDir = Join-Path $ProjectRoot ".venv"

if (-not $env:HOME) {
    $env:HOME = $env:USERPROFILE
}

function Test-VenvPython {
    param([string]$PythonPath)
    if (-not (Test-Path -LiteralPath $PythonPath)) {
        return $false
    }
    & $PythonPath -c "import sys" *> $null
    return $LASTEXITCODE -eq 0
}

function New-ProjectVenv {
    if (Get-Command uv -ErrorAction SilentlyContinue) {
        uv venv --python 3.11 .venv
        if ($LASTEXITCODE -eq 0) {
            return
        }
        Write-Warning "uv could not create Python 3.11 venv; falling back to python -m venv."
    }

    python -m venv .venv
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to create .venv with uv or python -m venv."
    }
}

function Install-ProjectRequirements {
    if (Get-Command uv -ErrorAction SilentlyContinue) {
        uv pip install --python $VenvPython -r requirements-win.txt
        if ($LASTEXITCODE -eq 0) {
            return
        }
        Write-Warning "uv could not install requirements; falling back to python -m pip."
    }

    & $VenvPython -m pip install -r requirements-win.txt
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to install requirements-win.txt."
    }
}

Push-Location $ProjectRoot
try {
    $NeedVenv = -not (Test-VenvPython $VenvPython)
    if ($NeedVenv) {
        if (Test-Path -LiteralPath $VenvDir) {
            Remove-Item -LiteralPath $VenvDir -Recurse -Force
        }
        New-ProjectVenv
    }

    if ($Install -or $NeedVenv) {
        Install-ProjectRequirements
    }

    $RootEnv = Join-Path $RepoRoot ".env"
    $RootEnvExample = Join-Path $RepoRoot ".env.example"
    if (-not (Test-Path -LiteralPath $RootEnv) -and (Test-Path -LiteralPath $RootEnvExample)) {
        Copy-Item -LiteralPath $RootEnvExample -Destination $RootEnv
        Write-Host "Created repository-root .env from .env.example. Fill tokens before OCR upload jobs."
    }

    $Scripts = @{
        worker    = "scripts\zotero_llm_worker.py"
        mineru    = "scripts\mineru_law_politics_markdown.py"
        html      = "scripts\pdf_to_html_paddleocr.py"
        inventory = "scripts\ten_to_50_pdf_paddle_inventory.py"
        normalize = "scripts\normalize_pdf_attachment_names.py"
        paddleocr = "scripts\paddleocr_vl_cli.py"
    }

    & $VenvPython $Scripts[$Task] @ScriptArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Task '$Task' failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}
