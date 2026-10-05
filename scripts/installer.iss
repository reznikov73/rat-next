; Rat Next — скрипт установщика (Inno Setup 6).
; Сборка: scripts\build-installer.ps1

#ifndef DateStamp
  #define DateStamp "dev"
#endif
#ifndef PayloadDir
  #define PayloadDir "..\dist\payload"
#endif

[Setup]
AppId={{7B54C1E2-8F3A-4D6B-9E21-3C5A8D4F6B90}
AppName=Rat Next
AppVersion={#DateStamp}
AppVerName=Rat Next ({#DateStamp})
AppPublisher=Rat Next
AppPublisherURL=https://github.com/reznikov73/rat-next
DefaultDirName={autopf}\Rat Next
DefaultGroupName=Rat Next
DisableProgramGroupPage=yes
; Обычный запуск — установка для текущего пользователя (без UAC).
; Запуск от администратора — можно поставить для всех пользователей.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=rat-next-setup-{#DateStamp}
SetupIconFile={#PayloadDir}\rat-next.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
UninstallDisplayName=Rat Next
UninstallDisplayIcon={app}\rat.exe

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Запускать Rat Next при входе в Windows (рекомендуется)"; GroupDescription: "Дополнительные параметры:"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "Дополнительные параметры:"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Excludes: "rat-next.ico"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Rat Next"; Filename: "{app}\rat.exe"
Name: "{autodesktop}\Rat Next"; Filename: "{app}\rat.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\rat.exe"; Parameters: "--install-autostart"; Tasks: autostart; Flags: runhidden; StatusMsg: "Включаю автозапуск…"
Filename: "{app}\rat.exe"; Description: "{cm:LaunchProgram,Rat Next}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\rat.exe"; Parameters: "--remove-autostart"; Flags: runhidden skipifdoesntexist; RunOnceId: "RemoveAutostart"

[UninstallDelete]
Type: filesandordirs; Name: "{app}"