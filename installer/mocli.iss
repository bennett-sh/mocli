; Inno Setup 6 script. Build it with scripts/package.ps1, which passes the
; version and the executable.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\mocli.exe"
#endif

#define AppName "mocli"
#define AppUrl "https://github.com/bennett-sh/mocli"
#define RunKey "Software\Microsoft\Windows\CurrentVersion\Run"
#define AppKey "Software\mocli"

[Setup]
; Identifies the app across versions: never change it.
AppId={{7B0E6A52-3C1F-4E8B-9D2A-5F64C0A9E1B3}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=bennett-sh
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
; Installs for the current user, so no administrator rights are needed.
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir=..\dist
OutputBaseFilename={#AppName}-{#AppVersion}-x64-setup
SetupIconFile=..\assets\mocli.ico
UninstallDisplayIcon={app}\mocli.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The running app is stopped by the code below.
CloseApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
english.CheckForUpdates=Check for updates once a day
german.CheckForUpdates=Einmal täglich nach Updates suchen

[Tasks]
Name: "startup"; Description: "{cm:AutoStartProgram,{#AppName}}"
; Only asked before the first start: afterwards the app's settings hold the choice.
Name: "updates"; Description: "{cm:CheckForUpdates}"; Check: IsFirstSetup
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "mocli.exe"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\mocli.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\mocli.exe"; Tasks: desktopicon

[Registry]
; The app's "Start with Windows" setting reads and writes the same value.
Root: HKCU; Subkey: "{#RunKey}"; ValueType: string; ValueName: "mocli"; ValueData: """{app}\mocli.exe"""; Tasks: startup
; Removed on uninstall, however it was created.
Root: HKCU; Subkey: "{#RunKey}"; ValueType: none; ValueName: "mocli"; Flags: uninsdeletevalue dontcreatekey
; Read and removed by the app at its next start, which saves it in its config.
Root: HKCU; Subkey: "{#AppKey}"; ValueType: dword; ValueName: "CheckForUpdates"; ValueData: 0; Tasks: not updates; Check: IsFirstSetup
Root: HKCU; Subkey: "{#AppKey}"; ValueType: none; Flags: uninsdeletekey dontcreatekey

[Run]
Filename: "{app}\mocli.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM mocli.exe"; Flags: runhidden; RunOnceId: "StopMocli"

[Code]
// Before the app's first start there are no settings to keep.
function IsFirstSetup: Boolean;
begin
  Result := not FileExists(ExpandConstant('{userappdata}\{#AppName}\config.toml'));
end;

// Replacing a running executable fails, and the history database survives being
// stopped at any moment.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM mocli.exe', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
