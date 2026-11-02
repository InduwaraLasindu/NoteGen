#ifndef AppVersion
  #define AppVersion "0.5.0"
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
Name: "localai"; Description: "Install local AI and all required models for this PC (internet; several GB)"; GroupDescription: "Ready to use AI:"; Flags: checkedonce
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#PayloadDir}\provenance\*"; DestDir: "{app}\provenance"; Flags: ignoreversion recursesubdirs createallsubdirs
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
WelcomeLabel2=Install NoteGen on this Windows account.%n%nPrinted OCR, PDF rendering, English language data and native dependencies are included. Leave local AI selected to download and verify a compatible AI runtime and all reader models needed for this PC inside this installer. This needs internet and several GB of disk space.%n%nYou can skip AI for an offline installation and install it later from Settings. Models, writer profiles and document projects are preserved during upgrades and uninstall.

[Code]
var
  AIPage: TOutputProgressWizardPage;
  InstallingAI, AIFailed: Boolean;
  AICancelFile: String;

procedure InitializeWizard;
begin
  AIPage := CreateOutputProgressPage('Setting up local AI',
    'Downloading, verifying and activating the AI required for this PC.');
end;

procedure AIOutput(const S: String; const Error, FirstLine: Boolean);
var
  Tail, Position, Detail: String;
  Separator, Value: Integer;
begin
  Log(S);
  if Copy(S, 1, 11) = 'NGPROGRESS|' then begin
    Tail := Copy(S, 12, Length(S));
    Separator := Pos('|', Tail);
    if Separator > 0 then begin
      Position := Copy(Tail, 1, Separator - 1);
      Detail := Copy(Tail, Separator + 1, Length(Tail));
      Value := StrToIntDef(Position, 0);
      if Value > 1000 then Value := 1000;
      AIPage.SetText(Detail, 'Partial downloads are kept so setup can resume. Cancel stops AI setup.');
      AIPage.SetProgress(Value, 1000);
    end;
  end else if Trim(S) <> '' then
    AIPage.SetText('Installing local AI', Copy(S, 1, 250));
end;

procedure CancelButtonClick(CurPageID: Integer; var Cancel, Confirm: Boolean);
begin
  if InstallingAI then begin
    SaveStringToFile(AICancelFile, 'cancel', False);
    AIPage.SetText('Stopping AI setup safely...', 'Verified files and resumable downloads will be kept.');
    Cancel := False;
    Confirm := False;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Params, Preset, Backend: String;
  Code: Integer;
  Started: Boolean;
begin
  if CurStep <> ssPostInstall then exit;
  { Silent/offline deployments never download unless explicitly requested. }
  if WizardSilent then begin
    if ExpandConstant('{param:AI|0}') <> '1' then exit;
  end else if not WizardIsTaskSelected('localai') then exit;
  Preset := ExpandConstant('{param:AIPRESET|maximum}');
  if (Preset <> 'fast') and (Preset <> 'balanced') and (Preset <> 'accuracy') and (Preset <> 'maximum') then Preset := 'maximum';
  Backend := ExpandConstant('{param:AIBACKEND|auto}');
  if (Backend <> 'auto') and (Backend <> 'cpu') and (Backend <> 'cuda') and (Backend <> 'vulkan') then Backend := 'auto';
  AICancelFile := ExpandConstant('{tmp}\notegen-cancel-ai');
  DeleteFile(AICancelFile);
  Params := '--installer-progress --cancel-file "' + AICancelFile + '" setup --preset ' + Preset + ' --backend ' + Backend + ' --output "' + ExpandConstant('{app}\ai-settings.json') + '"';
  AIPage.SetText('Detecting this PC and selecting compatible readers...', 'Internet is needed for publisher downloads.');
  AIPage.SetProgress(0, 1000);
  AIPage.Show;
  WizardForm.CancelButton.Enabled := True;
  InstallingAI := True;
  try
    Code := -1;
    Started := ExecAndLogOutput(ExpandConstant('{app}\notegen-cli.exe'), Params,
      ExpandConstant('{app}'), SW_SHOWNORMAL, ewWaitUntilTerminated, Code, @AIOutput);
    AIFailed := (not Started) or (Code <> 0);
    if AIFailed then begin
      Log('AI setup incomplete. Base application is installed; verified downloads are preserved.');
      if not WizardSilent then
        MsgBox('NoteGen is installed, but local AI setup did not finish. Open Settings > Updates & installation and click Install all required AI to resume. The installer log contains the error. Printed OCR is ready to use.', mbInformation, MB_OK);
    end else begin
      AIPage.SetText('AI is ready', 'The runtime and all selected reader models have been verified.');
      AIPage.SetProgress(1000, 1000);
      Log('NOTEGEN_AI_SETUP_VERIFIED');
    end;
  finally
    InstallingAI := False;
    AIPage.Hide;
  end;
end;

function GetCustomSetupExitCode: Integer;
begin
  if AIFailed then Result := 10 else Result := 0;
end;
