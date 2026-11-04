; Inno Setup script for AIO Cooler Screen. Build with build-installer.ps1.

#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#define AppName "AIO Cooler Screen"
#define AppExe "aio-screen.exe"
#define ServiceName "AioScreen"

[Setup]
AppId={{8F3C2B1E-6A4D-4C2B-9E1F-5D7A3B2C1E90}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=AIO Cooler Screen contributors
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup
VersionInfoCopyright=Copyright (c) 2026 Thnorty. MIT License.
; The service runs as SYSTEM from here, so it must stay in an admin-only folder.
DefaultDirName={autopf}\AIO Screen
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=AIO-Cooler-Screen-Setup-{#AppVersion}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
LicenseFile=..\LICENSE
; Follows the Windows light/dark setting (needs Inno Setup 6.6 or later).
WizardStyle=modern dynamic
ShowLanguageDialog=yes
Compression=lzma2/max
SolidCompression=yes
CloseApplications=no
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"

[CustomMessages]
english.DesktopShortcut=Create a desktop shortcut
english.DownloadFFmpeg=Download ffmpeg (about 90 MB) for video backgrounds
english.InstallingPawnIO=Installing the PawnIO sensor driver...
english.DownloadingFFmpeg=Downloading ffmpeg for video backgrounds...
english.StartingService=Installing and starting the background service...
english.LaunchApp=Launch AIO Cooler Screen
english.CloseFailed=The previous AIO Cooler Screen processes could not be stopped. Close the app and retry setup. Details:
english.Cleanup=Stopping AIO Cooler Screen and the sensor helper...
turkish.DesktopShortcut=Masaüstü kısayolu oluştur
turkish.DownloadFFmpeg=Video arka planları için ffmpeg indir (yaklaşık 90 MB)
turkish.InstallingPawnIO=PawnIO sensör sürücüsü kuruluyor...
turkish.DownloadingFFmpeg=Video arka planları için ffmpeg indiriliyor...
turkish.StartingService=Arka plan hizmeti kuruluyor ve başlatılıyor...
turkish.LaunchApp=AIO Cooler Screen uygulamasını aç
turkish.CloseFailed=Önceki AIO Cooler Screen işlemleri kapatılamadı. Uygulamayı kapatıp kurulumu tekrar deneyin. Ayrıntılar:
turkish.Cleanup=AIO Cooler Screen ve sensör yardımcısı kapatılıyor...

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopShortcut}"; Flags: unchecked
Name: "ffmpeg"; Description: "{cm:DownloadFFmpeg}"; Check: FfmpegMissing

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\sensor-probe-dist\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\installer\stop-aio-for-upgrade.ps1"; Flags: dontcopy
Source: "..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\licenses\MPL-2.0.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\assets\fonts\*.ttf"; DestDir: "{app}\fonts"; Flags: ignoreversion
Source: "..\assets\fonts\licenses\*.txt"; DestDir: "{app}\fonts\licenses"; Flags: ignoreversion
; PawnIO: signed open-source driver for reading CPU temperatures (installed only if missing).
Source: "..\third_party\PawnIO_setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: PawnIOMissing
Source: "..\third_party\pawnio\AMDFamily17.bin"; DestDir: "{app}\pawnio"; Flags: ignoreversion
Source: "..\third_party\pawnio\IntelMSR.bin"; DestDir: "{app}\pawnio"; Flags: ignoreversion
Source: "..\third_party\pawnio\COPYING"; DestDir: "{app}\pawnio"; DestName: "COPYING-modules.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{tmp}\PawnIO_setup.exe"; Parameters: "-install -silent"; StatusMsg: "{cm:InstallingPawnIO}"; Flags: waituntilterminated; Check: PawnIOMissing
Filename: "{app}\{#AppExe}"; Parameters: "fetch-ffmpeg"; StatusMsg: "{cm:DownloadingFFmpeg}"; Flags: runhidden waituntilterminated; Tasks: ffmpeg
Filename: "{app}\{#AppExe}"; Parameters: "install"; StatusMsg: "{cm:StartingService}"; Flags: runhidden waituntilterminated
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchApp}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\{#AppExe}"; Parameters: "uninstall --keep-files"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"

[UninstallDelete]
Type: filesandordirs; Name: "{app}\ffmpeg"

[Code]
{ PawnIO may already be installed by another tool (LibreHardwareMonitor, FanControl...). }
function PawnIOMissing(): Boolean;
begin
  Result := not FileExists(ExpandConstant('{commonpf64}\PawnIO\PawnIOLib.dll'));
end;

{ Offer the ffmpeg download only when no ffmpeg is installed already. }
function FfmpegMissing(): Boolean;
begin
  Result := (FileSearch('ffmpeg.exe', GetEnv('PATH')) = '') and
            not FileExists(ExpandConstant('{app}\ffmpeg\ffmpeg.exe')) and
            not FileExists(ExpandConstant('{localappdata}\Microsoft\WinGet\Links\ffmpeg.exe'));
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
  Args: String;
begin
  Result := '';
  ExtractTemporaryFile('stop-aio-for-upgrade.ps1');
  Args := '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' +
    ExpandConstant('{tmp}\stop-aio-for-upgrade.ps1') + '" -InstallDir "' +
    ExpandConstant('{app}') + '"';
  Log(ExpandConstant('{cm:Cleanup}'));
  if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    Args, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    Result := ExpandConstant('{cm:CloseFailed}') + ' (PowerShell could not start)'
  else if ResultCode <> 0 then
    Result := ExpandConstant('{cm:CloseFailed}') + ' (exit ' + IntToStr(ResultCode) + ')';
end;
