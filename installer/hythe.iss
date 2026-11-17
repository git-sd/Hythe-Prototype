; Hythe Windows installer (Inno Setup 6). Built by installer\build-installer.ps1:
;   ISCC.exe /DMyVersion=0.1.13 /DPayload=<folder with hythe.exe> installer\hythe.iss
; Installs for the current user (no admin), with the usual wizard.

#ifndef MyVersion
  #define MyVersion "0.0.0"
#endif
#ifndef Payload
  #define Payload "..\target\dist\release"
#endif

[Setup]
AppId={{6E1F4C5A-3B7D-4E8A-9C21-7A4D0B5E8F13}
AppName=Hythe
AppVersion={#MyVersion}
AppVerName=Hythe {#MyVersion}
AppPublisher=Hythe
AppPublisherURL=https://github.com/git-sd/Hythe-Prototype
AppSupportURL=https://github.com/git-sd/Hythe-Prototype/issues
AppUpdatesURL=https://github.com/git-sd/Hythe-Prototype/releases
DefaultDirName={userpf}\Hythe
DefaultGroupName=Hythe
DisableProgramGroupPage=no
DisableDirPage=no
DisableReadyPage=no
DisableWelcomePage=no
LicenseFile=license.txt
AllowNoIcons=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=Hythe-Setup-{#MyVersion}
SetupIconFile=..\assets\hythe.ico
UninstallDisplayIcon={app}\hythe.exe
WizardStyle=modern
WizardImageFile=wizard-side.bmp
WizardSmallImageFile=wizard-small.bmp
Compression=lzma2/max
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no
VersionInfoVersion={#MyVersion}.0
VersionInfoDescription=Hythe Setup

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#Payload}\hythe.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\hythe-agent.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Hythe"; Filename: "{app}\hythe.exe"; IconFilename: "{app}\hythe.exe"
Name: "{autodesktop}\Hythe"; Filename: "{app}\hythe.exe"; IconFilename: "{app}\hythe.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\hythe.exe"; Description: "Launch Hythe"; Flags: nowait postinstall skipifsilent
; After a silent self-update Hythe reopens by itself.
Filename: "{app}\hythe.exe"; Flags: nowait; Check: Relaunch

[UninstallDelete]
Type: filesandordirs; Name: "{app}\update"
Type: files; Name: "{app}\update-check.stamp"

[Code]
function Relaunch: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;

function GitInstalled: Boolean;
var Code: Integer;
begin
  Result := Exec(ExpandConstant('{cmd}'), '/C git --version', '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    // Remove what the old single-file setup (0.1.12 and the first 0.1.13 build) left behind.
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Hythe');
    DeleteFile(ExpandConstant('{app}\Uninstall.exe'));
  end;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpFinished) and not WizardSilent and not GitInstalled then
    WizardForm.FinishedLabel.Caption := WizardForm.FinishedLabel.Caption + #13#10#13#10 +
      'Git was not found. Hythe uses it for source control, threads and chat: install it from git-scm.com.';
end;
