#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif
#ifndef PayloadDir
  #error PayloadDir is required
#endif
#ifndef OutputPath
  #error OutputPath is required
#endif

[Setup]
AppId={{6D37E156-3C8D-4BB7-AAC8-8F6C4C8F331A}
AppName=NoteGen
AppVersion={#AppVersion}
AppVerName=NoteGen {#AppVersion}
AppPublisher=InduwaraLasindu
AppPublisherURL=https://github.com/InduwaraLasindu/NoteGen
AppSupportURL=https://github.com/InduwaraLasindu/NoteGen/issues
AppUpdatesURL=https://github.com/InduwaraLasindu/NoteGen/releases
DefaultDirName={localappdata}\Programs\NoteGen
DefaultGroupName=NoteGen
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
WizardStyle=modern dynamic
WizardSizePercent=110
DisableWelcomePage=no
UsePreviousAppDir=yes
UninstallDisplayIcon={app}\NoteGen.exe
UninstallDisplayName=NoteGen
OutputDir={#OutputPath}
OutputBaseFilename=NoteGen-Setup-{#AppVersion}
Compression=lzma2/normal
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
SetupLogging=yes
VersionInfoVersion={#AppVersion}.0
VersionInfoDescription=NoteGen Windows Installer
VersionInfoProductName=NoteGen

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\scripts\*"; DestDir: "{app}\scripts"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\runtime\pdfium.dll"; DestDir: "{app}\runtime"; Flags: ignoreversion
Source: "{#PayloadDir}\runtime\tesseract\*"; DestDir: "{app}\runtime\tesseract"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\runtime\*"; DestDir: "{app}\runtime"; Excludes: "pdfium.dll"; Flags: onlyifdoesntexist

[Icons]
Name: "{group}\NoteGen"; Filename: "{app}\NoteGen.exe"; WorkingDir: "{app}"
Name: "{group}\Set up local AI"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\scripts\Configure-AI.ps1"""; WorkingDir: "{app}"
Name: "{group}\Setup guide"; Filename: "{app}\docs\INSTALLER.txt"
Name: "{group}\Uninstall NoteGen"; Filename: "{uninstallexe}"
Name: "{autodesktop}\NoteGen"; Filename: "{app}\NoteGen.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\NoteGen.exe"; Description: "Launch NoteGen"; Flags: nowait postinstall skipifsilent

[Messages]
WelcomeLabel2=Install NoteGen on this Windows account.%n%nPrinted OCR and Word export are included. For local AI, use "Set up local AI" in the Start menu after installation. AI model downloads are separate and can be large.%n%nYour models and document projects are preserved during upgrades and uninstall.
