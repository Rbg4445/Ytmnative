; YTM Native — Windows Kurulum Sihirbazı (Inno Setup 6)
#define MyAppName "YTM Native"
#define MyAppVersion "7.1.0"
#define MyAppPublisher "Rbg4445"
#define MyAppURL "https://github.com/Rbg4445/Ytmnative"
#define MyAppExeName "ytm-native.exe"

[Setup]
AppId={{8D6B2A1E-5C4F-4E9B-B712-9A3E5F2C10A8}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
; Per-user install under %LOCALAPPDATA%\YTM Native so 5s auto-update never needs UAC/admin!
DefaultDirName={localappdata}\{#MyAppName}
DefaultGroupName={#MyAppName}
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=YTM-Native-Setup-{#MyAppVersion}
SetupIconFile=..\icons\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
; Standard ZIP compression avoids LZMA high-entropy heuristic false positives on VirusTotal
Compression=zip
SolidCompression=no
VersionInfoVersion=7.1.0.0
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription=YTM Native — Pure-Rust Desktop Client Installer
VersionInfoCopyright=Copyright (c) 2026 {#MyAppPublisher} (MIT License)
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion=7.1.0.0
VersionInfoProductTextVersion={#MyAppVersion}
WizardStyle=modern
CloseApplications=force
RestartApplications=no

[Languages]
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Masaüstüne kısayol oluştur"; GroupDescription: "Ek kısayollar:"
Name: "startupicon"; Description: "Windows açıldığında otomatik başlat"; GroupDescription: "Ek kısayollar:"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon
Name: "{userstartup}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: startupicon

[Registry]
; Register ytmd:// shared-link protocol (ytmdesktop2 compatibility)
Root: HKCU; Subkey: "Software\Classes\ytmd"; ValueType: string; ValueName: ""; ValueData: "URL:YTM Native Protocol"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\ytmd"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\ytmd\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"",0"
Root: HKCU; Subkey: "Software\Classes\ytmd\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{#MyAppName} uygulamasını şimdi başlat"; Flags: nowait postinstall skipifsilent
