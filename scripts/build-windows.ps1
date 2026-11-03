<#
.SYNOPSIS
  Compile Nova Download Manager et produit les installateurs Windows (.exe NSIS et .msi).

.DESCRIPTION
  1. Vérifie les prérequis (Rust, Node.js, outils C++ de Visual Studio, WebView2).
  2. Avec -InstallPrerequisites, installe ce qui manque via winget.
  3. Lance les tests du moteur, installe les dépendances JavaScript et compile l'application.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -InstallPrerequisites
  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -SkipTests
#>
param(
    [switch]$InstallPrerequisites,
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Desktop = Join-Path $Root "apps\desktop"

function Step($text) { Write-Host "`n==> $text" -ForegroundColor Cyan }
function Ok($text) { Write-Host "    OK  $text" -ForegroundColor Green }
function Missing($text) { Write-Host "    MANQUANT  $text" -ForegroundColor Yellow }

function Refresh-Path {
    $env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" +
                [Environment]::GetEnvironmentVariable("Path", "User")
    $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    if (Test-Path $cargoBin) { $env:Path = "$cargoBin;$env:Path" }
}

function Test-Command($name) { [bool](Get-Command $name -ErrorAction SilentlyContinue) }

function Test-VCTools {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { return $false }
    $path = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    return [bool]$path
}

function Test-WebView2 {
    $keys = @(
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        "HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
    )
    foreach ($k in $keys) { if (Test-Path $k) { return $true } }
    return $false
}

function Winget-Install($id, $extra = @()) {
    if (-not (Test-Command winget)) {
        throw "winget est introuvable. Installez « App Installer » depuis le Microsoft Store, ou installez $id manuellement."
    }
    Write-Host "    Installation de $id..."
    winget install --id $id -e --accept-source-agreements --accept-package-agreements @extra
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne -1978335189) { # -1978335189 = déjà installé
        throw "L'installation de $id a échoué (code $LASTEXITCODE)."
    }
    Refresh-Path
}

# ---------------------------------------------------------------------------
Step "Vérification des prérequis"
Refresh-Path
$missing = @()

if (Test-Command cargo) { Ok "Rust $(cargo --version)" } else { Missing "Rust (rustup)"; $missing += "rust" }

if (Test-Command node) {
    $nodeMajor = [int]((node --version).TrimStart("v").Split(".")[0])
    if ($nodeMajor -ge 20) { Ok "Node.js $(node --version)" } else { Missing "Node.js 20 ou plus (trouvé $(node --version))"; $missing += "node" }
} else { Missing "Node.js 20 ou plus"; $missing += "node" }

if (Test-VCTools) { Ok "Outils de compilation C++ (MSVC)" } else { Missing "Visual Studio Build Tools, charge de travail C++"; $missing += "msvc" }

if (Test-WebView2) { Ok "Microsoft Edge WebView2" } else { Missing "WebView2 Runtime (présent par défaut sous Windows 11)"; $missing += "webview2" }

if ($missing.Count -gt 0) {
    if (-not $InstallPrerequisites) {
        Write-Host "`nPrérequis manquants. Relancez avec -InstallPrerequisites pour les installer automatiquement :" -ForegroundColor Yellow
        Write-Host "  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -InstallPrerequisites"
        exit 1
    }
    Step "Installation des prérequis manquants"
    if ($missing -contains "msvc") {
        Winget-Install "Microsoft.VisualStudio.2022.BuildTools" @("--override", "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended")
    }
    if ($missing -contains "rust") {
        Winget-Install "Rustlang.Rustup"
        rustup default stable
    }
    if ($missing -contains "node") { Winget-Install "OpenJS.NodeJS.LTS" }
    if ($missing -contains "webview2") { Winget-Install "Microsoft.EdgeWebView2Runtime" }
    Refresh-Path
    if (-not (Test-Command cargo) -or -not (Test-Command node)) {
        Write-Host "`nInstallation terminée. Fermez et rouvrez PowerShell, puis relancez ce script." -ForegroundColor Yellow
        exit 0
    }
}

rustup target add x86_64-pc-windows-msvc | Out-Null

# ---------------------------------------------------------------------------
Push-Location $Root
try {
    if (-not $SkipTests) {
        Step "Tests du moteur de téléchargement"
        cargo test -p ndm-engine --release
        if ($LASTEXITCODE -ne 0) { throw "Des tests du moteur ont échoué." }
        Ok "Tests réussis"
    }

    Step "Dépendances JavaScript"
    Push-Location $Desktop
    npm ci
    if ($LASTEXITCODE -ne 0) { throw "npm ci a échoué." }

    Step "Compilation de l'application et des installateurs (plusieurs minutes la première fois)"
    npx tauri build
    if ($LASTEXITCODE -ne 0) { throw "La compilation Tauri a échoué." }
    Pop-Location

    Step "Terminé"
    $bundle = Join-Path $Root "target\release\bundle"
    Get-ChildItem -Recurse -Path $bundle -Include *.exe, *.msi | ForEach-Object {
        Write-Host ("    {0}  ({1:N1} Mo)" -f $_.FullName, ($_.Length / 1MB)) -ForegroundColor Green
    }
    Write-Host "`nApplication seule : $(Join-Path $Root 'target\release\nova-download-manager.exe')"
}
finally {
    Pop-Location
}
