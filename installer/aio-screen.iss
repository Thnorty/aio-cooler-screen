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
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "ffmpeg"; Description: "Download ffmpeg (about 90 MB) for video backgrounds"; Check: FfmpegMissing

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
; PawnIO: signed open-source driver for reading CPU temperatures (installed only if missing).
Source: "..\third_party\PawnIO_setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: PawnIOMissing
Source: "..\third_party\pawnio\AMDFamily17.bin"; DestDir: "{app}\pawnio"; Flags: ignoreversion
Source: "..\third_party\pawnio\IntelMSR.bin"; DestDir: "{app}\pawnio"; Flags: ignoreversion
Source: "..\third_party\pawnio\COPYING"; DestDir: "{app}\pawnio"; DestName: "COPYING-modules.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{tmp}\PawnIO_setup.exe"; Parameters: "-install -silent"; StatusMsg: "Installing the PawnIO sensor driver..."; Flags: waituntilterminated; Check: PawnIOMissing
Filename: "{app}\{#AppExe}"; Parameters: "fetch-ffmpeg"; StatusMsg: "Downloading ffmpeg for video backgrounds..."; Flags: runhidden waituntilterminated; Tasks: ffmpeg
Filename: "{app}\{#AppExe}"; Parameters: "install"; StatusMsg: "Installing and starting the background service..."; Flags: runhidden waituntilterminated
Filename: "{app}\{#AppExe}"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent

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
begin
  { Stop the running service so its executable can be replaced; net stop waits for it. }
  Exec(ExpandConstant('{sys}\net.exe'), 'stop {#ServiceName}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
