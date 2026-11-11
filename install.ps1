# ---------------------------------------------------------------------------
#  Keyhold - installer for Windows
#
#  Builds the interface bundle, compiles the binary with it embedded, puts it in
#  %LOCALAPPDATA%\Programs\Keyhold (on your PATH), adds Keyhold to the Start menu
#  and makes keyhold:// links open it.
#
#      powershell -ExecutionPolicy Bypass -File .\install.ps1
#      powershell -ExecutionPolicy Bypass -File .\install.ps1 -SkipBuild
#
#  -SkipBuild     use the interface already built in web\dist
#  -SkipShortcut  no Start menu entry and no keyhold:// link
#
#  Re-running upgrades in place.
# ---------------------------------------------------------------------------
param(
    [switch]$SkipBuild,
    [switch]$SkipShortcut
)

$ErrorActionPreference = 'Stop'
$Source = $PSScriptRoot
$BinDir = Join-Path $env:LOCALAPPDATA 'Programs\Keyhold'
$Exe = Join-Path $BinDir 'keyhold.exe'
$StateDir = Join-Path $env:LOCALAPPDATA 'Keyhold\state'

function Info($text) { Write-Host "  * $text" -ForegroundColor Cyan }
function Ok($text) { Write-Host "  * $text" -ForegroundColor Green }
function Warn($text) { Write-Host "  * $text" -ForegroundColor Yellow }
function Fail($text) { Write-Host "  * $text" -ForegroundColor Red; exit 1 }

Write-Host ''
Write-Host '  Keyhold - installer' -ForegroundColor White
Write-Host ''

Info 'Checking the toolchain...'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Fail 'cargo not found. Install Rust: rustup.rs' }
if (-not $SkipBuild) {
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) { Fail 'Node.js (npm) is required to build the interface.' }
}

if ($SkipBuild) {
    if (-not (Test-Path (Join-Path $Source 'web\dist\index.html'))) { Fail 'web\dist is empty; run without -SkipBuild.' }
    Warn '-SkipBuild: using the bundle already in web\dist'
} else {
    Info 'Building the interface...'
    Push-Location (Join-Path $Source 'web')
    npm install --silent --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { Fail 'npm install failed' }
    npm run build | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail 'the interface did not build' }
    Pop-Location
    Ok 'interface built'
}

Info 'Compiling Keyhold (this takes a minute the first time)...'
Push-Location $Source
cargo build --release --quiet
if ($LASTEXITCODE -ne 0) { Fail 'cargo build failed' }
Pop-Location

# Windows will not replace a program file that is running. Closing Keyhold locks
# the vault and forgets the key, so it is asked, never done behind your back.
$running = Get-Process keyhold -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe }
if ($running) {
    $answer = Read-Host '  Keyhold is open. Close it to upgrade? The vault locks. [y/N]'
    if ($answer -notmatch '^(y|yes|s|sim)$') { Fail 'Keyhold is still open; nothing was changed.' }
    $running | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
Copy-Item (Join-Path $Source 'target\release\keyhold.exe') $Exe -Force
Ok "installed $Exe"

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $BinDir) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $BinDir).TrimStart(';'), 'User')
    Ok "added $BinDir to your PATH (new terminals see it)"
}

if (-not $SkipShortcut) {
    New-Item -ItemType Directory -Force -Path $StateDir | Out-Null

    # A hidden PowerShell starts Keyhold without a console window, and keeps its
    # output (which carries the session address) in a log only you can read.
    $log = Join-Path $StateDir 'app.log'
    $launch = "-NoProfile -WindowStyle Hidden -Command `"Start-Process -FilePath '$Exe' -ArgumentList '--present' -WindowStyle Hidden -RedirectStandardOutput '$log' -RedirectStandardError '$log.err'`""

    # The icon, as an .ico made from the PNG.
    $icon = Join-Path $BinDir 'keyhold.ico'
    try {
        Add-Type -AssemblyName System.Drawing
        $png = [System.Drawing.Bitmap]::FromFile((Join-Path $Source 'assets\icon.png'))
        $small = New-Object System.Drawing.Bitmap $png, 256, 256
        $handle = $small.GetHicon()
        $ico = [System.Drawing.Icon]::FromHandle($handle)
        $stream = [System.IO.File]::Create($icon)
        $ico.Save($stream)
        $stream.Close()
        $png.Dispose()
    } catch {
        $icon = $Exe
    }

    $menu = Join-Path ([Environment]::GetFolderPath('Programs')) 'Keyhold.lnk'
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($menu)
    $link.TargetPath = (Get-Command powershell).Source
    $link.Arguments = $launch
    $link.IconLocation = $icon
    $link.Description = 'A password vault that lives entirely on this machine'
    $link.Save()
    Ok 'Start menu entry installed'

    # keyhold:// opens Keyhold (start, or bring the running one forward). Any page
    # can link to it, and that is all a page gets: the address is never passed on.
    $classes = 'HKCU:\Software\Classes\keyhold'
    New-Item -Path "$classes\shell\open\command" -Force | Out-Null
    Set-ItemProperty -Path $classes -Name '(Default)' -Value 'URL:Keyhold'
    Set-ItemProperty -Path $classes -Name 'URL Protocol' -Value ''
    Set-ItemProperty -Path "$classes\shell\open\command" -Name '(Default)' -Value "`"$((Get-Command powershell).Source)`" $launch"
    Ok 'keyhold:// links open Keyhold'
}

Write-Host ''
Write-Host '  Done. Open Keyhold from the Start menu, or run keyhold in a new terminal.' -ForegroundColor White
Write-Host ''
