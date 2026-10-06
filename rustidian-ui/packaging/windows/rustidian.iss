; Rustidian — script de Inno Setup para el instalador de Windows.
;
; Compilar con:  iscc /DMyAppVersion=0.2.0 /DSourceDir=. rustidian.iss
; Usa los iconos de packaging/icons/windows: el tema claro usa el blanco y el
; oscuro el negro, tanto para el instalador como para los accesos directos.

#define MyAppName "Rustidian"
#define MyAppPublisher "Hu-Tao128"
#define MyAppURL "https://github.com/Hu-Tao128/rustidian"
#define MyAppExeName "rustidian-ui.exe"
#define IconDir "rustidian-ui\packaging\icons\windows"

#ifndef MyAppVersion
  #define MyAppVersion "0.2.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\.."
#endif

[Setup]
AppId={{B7B0E0C8-3C7B-4E2F-9D2A-6F1C4A9E0B11}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile={#SourceDir}\LICENSE
OutputDir={#SourceDir}
OutputBaseFilename=RustidianSetup-{#MyAppVersion}-x86_64
SetupIconFile={#SourceDir}\{#IconDir}\rustidian_black.ico
UninstallDisplayIcon={app}\rustidian_black.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\{#IconDir}\rustidian_black.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\{#IconDir}\rustidian_white.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{code:GetAppIcon}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{code:GetAppIcon}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
{ true si Windows está en tema claro (AppsUseLightTheme = 1). }
function IsLightTheme(): Boolean;
var
  Value: Cardinal;
begin
  Result := True;
  if RegQueryDWordValue(HKCU,
       'Software\Microsoft\Windows\CurrentVersion\Themes\Personalize',
       'AppsUseLightTheme', Value) then
    Result := (Value <> 0);
end;

{ Icono del acceso directo según el tema: blanco en claro, negro en oscuro. }
function GetAppIcon(Param: String): String;
begin
  if IsLightTheme() then
    Result := ExpandConstant('{app}\rustidian_white.ico')
  else
    Result := ExpandConstant('{app}\rustidian_black.ico');
end;
