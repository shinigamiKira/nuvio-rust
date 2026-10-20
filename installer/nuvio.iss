; Inno Setup script for the Nuvio (Rust) installer.
; Build with:  ISCC /DAppVersion=0.1.0 /DSourceDir=<folder with nuvio.exe and libmpv-2.dll> /DOutputDir=<dist dir> nuvio.iss

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #error SourceDir must point at the folder that holds nuvio.exe and libmpv-2.dll
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

#define AppName "Nuvio (Rust)"
#define AppExe "nuvio.exe"

[Setup]
; A different AppId from the Kotlin build, so the two can be installed side by side.
AppId={{5C7E1B94-2D3A-4E6F-8A15-B09C4D7E2F63}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Nuvio (unofficial Windows port)
AppPublisherURL=https://github.com/NuvioMedia/NuvioMobile
AppSupportURL=https://github.com/NuvioMedia/NuvioMobile/issues
DefaultDirName={autopf}\Nuvio Rust
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=Nuvio-Rust-Setup-{#AppVersion}
SetupIconFile=..\crates\app\resources\nuvio.ico
UninstallDisplayIcon={app}\{#AppExe}
LicenseFile=..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} installer

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// The library, watch progress and settings live outside the install folder. Keep them unless the user opts in.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if not UninstallSilent then
    begin
      if MsgBox('Also remove your library, watch progress, settings and cached images?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      begin
        DelTree(ExpandConstant('{userappdata}\NuvioRust'), True, True, True);
        DelTree(ExpandConstant('{localappdata}\NuvioRust'), True, True, True);
      end;
    end;
  end;
end;
