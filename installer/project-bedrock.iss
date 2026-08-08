; Inno Setup script for Project Bedrock.
;
; Produces a normal Windows installer: Start-menu entry, optional desktop
; icon, an uninstaller in Add/Remove Programs, and the Blender add-on placed
; somewhere findable rather than buried in a source tree.
;
; Built by .github/workflows/release.yml on every version tag. To build it by
; hand, install Inno Setup and run:
;     iscc /DAppVersion=0.1.0 installer\project-bedrock.iss
; from the repository root, with target\release\project-bedrock.exe already
; built.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

#define AppName "Project Bedrock"
; The shipped name, which is what the user sees and double-clicks.
; The build output keeps cargo's name; it is renamed on the way in.
#define AppExe "Project Bedrock.exe"
#define BuiltExe "project-bedrock.exe"
#define AddonFile "project_bedrock_import_tools.py"

[Setup]
AppId={{8C2F1E4A-9B3D-4A7E-8F21-5D6C0B9E4A11}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
OutputDir=..\dist
OutputBaseFilename=ProjectBedrock-{#AppVersion}-Setup
Compression=lzma2/max
SolidCompression=yes
; Per-user install by default so no admin prompt is needed; the app writes
; only to its own settings folder and never to Program Files.
PrivilegesRequiredOverridesAllowed=dialog
PrivilegesRequired=lowest
WizardStyle=modern
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\{#AddonFile}"; DestDir: "{app}\Blender Add-on"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\Blender Add-on"; Filename: "{app}\Blender Add-on"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName}"; Flags: nowait postinstall skipifsilent

[Messages]
; The add-on cannot be installed for the user -- Blender has to do that from
; its own preferences -- so say where it went instead of leaving them hunting.
FinishedLabel=Project Bedrock is installed.%n%nTo import your exports into Blender, open Blender and go to Edit > Preferences > Add-ons > Install, then choose:%n%n    {app}\Blender Add-on\project_bedrock_import_tools.py
