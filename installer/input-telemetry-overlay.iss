; The Windows installer (Inno Setup 6), built by installer/build.ps1 after
; `cargo build --release`. It installs for the current Windows user, without asking for
; administrator rights: a Start menu shortcut, an uninstaller in Settings > Apps, and
; "Open with" for Garage 61 CSVs that leaves whatever opens CSVs now as it is.

#ifndef Version
  #define Version "0.0.0"
#endif
#define Name "Input Telemetry Overlay"
#define Exe "input-telemetry-overlay.exe"
; "Open with" entry for CSVs.
#define ProgId "InputTelemetryOverlay.Lap"

[Setup]
; Never change it: every version's installer shares it, so a newer one upgrades an older.
AppId={{A7D27628-42AE-4F92-BC37-640F8E7FBA9D}
AppName={#Name}
AppVersion={#Version}
AppVerName={#Name} {#Version}
AppPublisher=Mitch Warrenburg
AppPublisherURL=https://github.com/mitchwarrenburg/input-telemetry-overlay
AppSupportURL=https://github.com/mitchwarrenburg/input-telemetry-overlay/issues
AppUpdatesURL=https://github.com/mitchwarrenburg/input-telemetry-overlay/releases
VersionInfoVersion={#Version}
; Paths below are from the repository root.
SourceDir=..
DefaultDirName={autopf}\{#Name}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; An overlay left running is closed before its files are replaced.
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes
SetupIconFile=assets\icon\icon.ico
UninstallDisplayIcon={app}\{#Exe}
UninstallDisplayName={#Name}
WizardStyle=modern
OutputBaseFilename=input-telemetry-overlay-v{#Version}-windows-x64-setup
Compression=lzma2
SolidCompression=yes

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "target\release\{#Exe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "assets\fonts\OFL.txt"; DestDir: "{app}"; DestName: "FONT-LICENSE-OFL.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#Name}"; Filename: "{app}\{#Exe}"
Name: "{autodesktop}\{#Name}"; Filename: "{app}\{#Exe}"; Tasks: desktopicon

[Registry]
; A CSV opened with the overlay is added to its lap library and becomes the reference.
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueData: "Garage 61 lap"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueData: """{app}\{#Exe}"",0"
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueData: """{app}\{#Exe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.csv\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#Exe}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#Name}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\{#Exe}\SupportedTypes"; ValueType: string; ValueName: ".csv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\Applications\{#Exe}\shell\open\command"; ValueType: string; ValueData: """{app}\{#Exe}"" ""%1"""

[Run]
Filename: "{app}\{#Exe}"; Description: "{cm:LaunchProgram,{#Name}}"; Flags: nowait postinstall skipifsilent
