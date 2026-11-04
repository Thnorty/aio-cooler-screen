# Elevated by Inno Setup. Scope all process termination to the chosen installation folder.
param([Parameter(Mandatory=$true)][string]$InstallDir)
$ErrorActionPreference = 'Stop'
$folder = [System.IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
function Installed-Processes {
    @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
        $_.Name -in @('aio-screen.exe','SensorProbe.exe') -and
        $_.ExecutablePath -and
        [string]::Equals([System.IO.Path]::GetDirectoryName($_.ExecutablePath),$folder,[StringComparison]::OrdinalIgnoreCase)
    })
}
try {
    # Stop the UI first; it can be launched with --tray and would otherwise hold aio-screen.exe open.
    $gui = @(Installed-Processes | Where-Object { $_.Name -eq 'aio-screen.exe' -and $_.CommandLine -notmatch '(?i)(^|\s)service(\s|$)' })
    foreach ($proc in $gui) {
        $handle = Get-Process -Id $proc.ProcessId -ErrorAction SilentlyContinue
        if ($handle) { [void]$handle.CloseMainWindow() }
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(6)
    while ((@(Installed-Processes | Where-Object { $_.ProcessId -in $gui.ProcessId })).Count -gt 0 -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 250 }
    # Stop the Windows service; allow its normal cleanup to dispose the SensorProbe child.
    $svc = Get-Service -Name 'AioScreen' -ErrorAction SilentlyContinue
    if ($svc -and $svc.Status -ne 'Stopped') {
        Stop-Service -Name 'AioScreen' -Force -ErrorAction Stop
        $svc.WaitForStatus('Stopped',[TimeSpan]::FromSeconds(18))
    }
    # An orphan can hold BlackSharp.Core.dll locked after the service is gone.
    foreach ($proc in @(Installed-Processes)) {
        Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while (@(Installed-Processes).Count -gt 0 -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 250 }
    $remaining = @(Installed-Processes)
    if ($remaining.Count -gt 0) { throw "Processes remain: $($remaining.ProcessId -join ', ')" }
    exit 0
} catch {
    Write-Error $_
    exit 1
}
