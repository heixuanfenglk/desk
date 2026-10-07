; 案头桌面控件的 Inno Setup 安装脚本。
; 日常打包直接双击 pack.bat，或运行 pack.ps1。

#define AppName "案头"
#define AppVersion "0.1.0"
#define AppExe "desk.exe"

[Setup]
AppId={{B7E4A1C2-6D58-4F93-9A20-1C8E5D3F7B64}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
VersionInfoVersion=0.1.0.0
VersionInfoProductName={#AppName}
DefaultDirName={localappdata}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=target\installer
OutputBaseFilename=desk-setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupLogging=yes
UninstallDisplayName={#AppName}
SetupIconFile=assets\desk.ico
UninstallDisplayIcon={app}\{#AppExe}
CloseApplications=force
CloseApplicationsFilter={#AppExe}
RestartApplications=no

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加选项:"; Flags: checkedonce
Name: "startup"; Description: "登录 Windows 时自动启动"; GroupDescription: "附加选项:"; Flags: checkedonce

[Files]
Source: "target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\卸载 {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon
Name: "{userstartup}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: startup

[Run]
Filename: "{app}\{#AppExe}"; Description: "启动{#AppName}"; Flags: nowait postinstall skipifsilent
