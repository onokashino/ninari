; The Windows installer, built by scripts/package.ps1: ninari for the current user only, in its
; own folder, so no admin rights are asked. Version, Source, Output and Icon come from iscc's /D.

[Setup]
AppId={{6E1A2B7C-3D44-4F0B-9C55-8A1E0F2B6D31}
AppName=ninari
AppVersion={#Version}
AppPublisher=onokashino
AppPublisherURL=https://onokashino.github.io/ninari/
AppSupportURL=https://github.com/onokashino/ninari/issues
DefaultDirName={localappdata}\Programs\ninari
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Output}
OutputBaseFilename=ninari-{#Version}-windows-x86_64-setup
SetupIconFile={#Icon}
UninstallDisplayIcon={app}\ninari.exe
LicenseFile={#Source}\LICENSE.txt
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\ninari"; Filename: "{app}\ninari.exe"; AppUserModelID: "onokashino.ninari"
Name: "{autodesktop}\ninari"; Filename: "{app}\ninari.exe"; Tasks: desktopicon

[Registry]
; ninari:// links from the plugin catalogue; the app writes the same on start.
Root: HKCU; Subkey: "Software\Classes\ninari"; ValueType: string; ValueName: ""; ValueData: "URL:ninari"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\ninari"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\ninari\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\ninari.exe"" ""%1"""

[Run]
Filename: "{app}\ninari.exe"; Description: "{cm:LaunchProgram,ninari}"; Flags: nowait postinstall skipifsilent
; An update from inside the app runs this installer quietly: ninari starts again by itself.
Filename: "{app}\ninari.exe"; Flags: nowait; Check: WizardSilent
