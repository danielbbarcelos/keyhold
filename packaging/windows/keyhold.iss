; Keyhold installer for Windows (Inno Setup 6). Built by .github/workflows/release.yml:
;   ISCC.exe /DAppVersion=1.2.3 packaging\windows\keyhold.iss
; Installs for the current user only (no administrator), in %LOCALAPPDATA%\Programs\Keyhold.
; The vault itself lives in %LOCALAPPDATA%\Keyhold and is never touched by installing,
; upgrading or uninstalling.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F2C8E41-9B3D-4A57-8E2F-4C1D7B9A0E35}
AppName=Keyhold
AppVersion={#AppVersion}
AppVerName=Keyhold {#AppVersion}
AppPublisher=Keyhold
AppPublisherURL=https://github.com/danielbbarcelos/keyhold
AppSupportURL=https://github.com/danielbbarcelos/keyhold/issues
AppUpdatesURL=https://github.com/danielbbarcelos/keyhold/releases
VersionInfoVersion={#AppVersion}
VersionInfoProductVersion={#AppVersion}
VersionInfoProductName=Keyhold
VersionInfoCompany=Keyhold
VersionInfoDescription=Instalador do Keyhold
VersionInfoCopyright=MIT License
DefaultDirName={localappdata}\Programs\Keyhold
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=KeyholdSetup
SetupIconFile=keyhold.ico
UninstallDisplayIcon={app}\keyhold.ico
UninstallDisplayName=Keyhold
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Upgrading over a running Keyhold: close it (the vault locks), then replace the files.
CloseApplications=force
RestartApplications=no
ChangesEnvironment=yes

[Languages]
Name: "pt"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "Criar um atalho na área de trabalho"; Flags: unchecked

[Files]
Source: "..\..\target\release\keyhold.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\keyhold-launch.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "keyhold.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Keyhold"; Filename: "{app}\keyhold-launch.exe"; IconFilename: "{app}\keyhold.ico"
Name: "{userdesktop}\Keyhold"; Filename: "{app}\keyhold-launch.exe"; IconFilename: "{app}\keyhold.ico"; Tasks: desktopicon

[Registry]
; keyhold:// opens Keyhold (start, or bring the running one forward). That is all a page gets.
Root: HKCU; Subkey: "Software\Classes\keyhold"; ValueType: string; ValueName: ""; ValueData: "URL:Keyhold"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\keyhold"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\keyhold\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\keyhold.ico"
Root: HKCU; Subkey: "Software\Classes\keyhold\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\keyhold-launch.exe"""
; `keyhold` on the PATH, for the command line and for a coding assistant's MCP setup.
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\keyhold-launch.exe"; Description: "Abrir o Keyhold"; Flags: postinstall nowait skipifsilent

[Code]
function NeedsAddPath(Dir: string): Boolean;
var
  Current: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Current) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Current) + ';') = 0;
end;
