# Builds aio-screen in release mode and packages it with Inno Setup into dist\.
# Downloads pinned third-party pieces (PawnIO driver installer and modules) into third_party\.
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

# --- Third-party: PawnIO (signed open-source driver used for CPU temperatures) ----------------
$PawnIOSetupUrl = 'https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe'
$ModulesUrl = 'https://github.com/namazso/PawnIO.Modules/releases/download/0.2.11/release_0_2_11.zip'
$ModulesSha256 = '43608CB89BC84247FEF1368A139013F7D043E17DB6D6C8DFC9B46BF0905A81F4'
$tp = Join-Path $PSScriptRoot 'third_party'
New-Item -ItemType Directory -Force "$tp\pawnio" | Out-Null

if (-not (Test-Path "$tp\PawnIO_setup.exe")) {
    Invoke-WebRequest $PawnIOSetupUrl -OutFile "$tp\PawnIO_setup.exe" -UseBasicParsing
}
$sig = Get-AuthenticodeSignature "$tp\PawnIO_setup.exe"
if ($sig.Status -ne 'Valid') {
    Remove-Item "$tp\PawnIO_setup.exe"
    Write-Error "PawnIO_setup.exe signature is not valid ($($sig.Status)); refusing to package it."
}

if (-not (Test-Path "$tp\pawnio\AMDFamily17.bin") -or -not (Test-Path "$tp\pawnio\IntelMSR.bin")) {
    $zip = "$tp\pawnio-modules.zip"
    Invoke-WebRequest $ModulesUrl -OutFile $zip -UseBasicParsing
    if ((Get-FileHash $zip -Algorithm SHA256).Hash -ne $ModulesSha256) {
        Remove-Item $zip
        Write-Error 'PawnIO modules download does not match the pinned SHA-256.'
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $z = [IO.Compression.ZipFile]::OpenRead($zip)
    foreach ($n in 'AMDFamily17.bin', 'IntelMSR.bin', 'COPYING') {
        [IO.Compression.ZipFileExtensions]::ExtractToFile($z.GetEntry($n), "$tp\pawnio\$n", $true)
    }
    $z.Dispose()
    Remove-Item $zip
}

# --- Fonts (OFL, official Google Fonts repository) --------------------------------------------
& "$PSScriptRoot\tools\fetch-fonts.ps1"
if ($LASTEXITCODE) { exit $LASTEXITCODE }

# --- App --------------------------------------------------------------------------------------
if (-not (Test-Path assets\icon.ico)) {
    cargo build --release
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
    Start-Process -FilePath target\release\aio-screen.exe -ArgumentList 'make-icon', 'assets' -Wait -WorkingDirectory $PSScriptRoot
}
# Built after the icon exists so the exe embeds it.
cargo build --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }

# --- Installer --------------------------------------------------------------------------------
$iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($cmd) { $iscc = $cmd.Source }
}
if (-not $iscc) {
    Write-Error "Inno Setup 6 not found. Install it from https://jrsoftware.org/isdl.php (or: winget install JRSoftware.InnoSetup), then rerun."
}

& $iscc "/DAppVersion=$version" installer\aio-screen.iss
if ($LASTEXITCODE) { exit $LASTEXITCODE }
Get-ChildItem dist\*.exe | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } }
