; Inno Setup script for the Windows installer (built by .github/workflows/release.yml).
; iscc /DAppVersion=0.1.0 /DSourceExe=path\to\Mugetsu.exe /DOutputDir=dist packaging\windows\mugetsu.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6C1F3E0A-4B7D-4E5B-9A51-0D3F6E2B7A11}
AppName=Mugetsu
AppVersion={#AppVersion}
AppPublisher=Mugetsu
DefaultDirName={autopf}\Mugetsu
DefaultGroupName=Mugetsu
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir={#OutputDir}
OutputBaseFilename=Mugetsu-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\Mugetsu.exe
SetupIconFile=..\..\crates\mugetsu-app\assets\icon\mugetsu.ico
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Mugetsu.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Mugetsu"; Filename: "{app}\Mugetsu.exe"
Name: "{autodesktop}\Mugetsu"; Filename: "{app}\Mugetsu.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Mugetsu.exe"; Description: "{cm:LaunchProgram,Mugetsu}"; Flags: nowait postinstall skipifsilent
