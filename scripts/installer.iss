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

[Code]
var
  RatWasRunning: Boolean;
  RelayWasRunning: Boolean;

function ServiceIsRunning(Name: String): Boolean;
var
  ResultCode: Integer;
begin
  { Определяем, запущена ли служба (sc query + поиск строки RUNNING). }
  Result := Exec(
    ExpandConstant('{cmd}'),
    '/c sc query ' + Name + ' | findstr /C:"RUNNING" >nul',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

procedure WaitStopped(Name: String);
var
  I: Integer;
begin
  for I := 1 to 20 do
  begin
    if not ServiceIsRunning(Name) then
      Break;
    Sleep(250);
  end;
end;

procedure StopOldServices;
var
  ResultCode: Integer;
begin
  { Обновление поверх работающей службы: запоминаем состояние, останавливаем,
    чтобы заменить файлы; после установки службы поднимаются обратно. }
  RatWasRunning := ServiceIsRunning('RatNext');
  RelayWasRunning := ServiceIsRunning('RatNextRelay');
  if RatWasRunning then
  begin
    Exec('sc.exe', 'stop RatNext', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    WaitStopped('RatNext');
  end;
  if RelayWasRunning then
  begin
    Exec('sc.exe', 'stop RatNextRelay', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    WaitStopped('RatNextRelay');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssInstall then
    StopOldServices;
  if CurStep = ssPostInstall then
  begin
    { Службы, работавшие до обновления, поднимаем обратно. }
    if RatWasRunning then
      Exec('sc.exe', 'start RatNext', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    if RelayWasRunning then
      Exec('sc.exe', 'start RatNextRelay', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;
end;

[Types]
Name: "default"; Description: "Обычная установка (Rat Next)"
Name: "custom"; Description: "Выборочная установка"; Flags: iscustom

[Components]
Name: "rat"; Description: "Rat Next — приложение (плитки, подключения, удалённый экран)"; Types: default
Name: "relay"; Description: "Relay-сервер (интернет-режим; обычно только на сервере)"

[Tasks]
; Службы больше не предлагаются в установщике: их можно включить в настройках
; самого приложения (Rat Next / relay) — Windows запросит права при операции.
Name: "autostart"; Description: "Запускать Rat Next при входе в Windows (с правами администратора, без службы)"; GroupDescription: "Дополнительные параметры:"; Flags: unchecked; Components: rat
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "Дополнительные параметры:"; Flags: unchecked; Components: rat
Name: "relay_autostart"; Description: "Запускать relay при входе в Windows (с правами администратора, без службы)"; GroupDescription: "Дополнительные параметры:"; Flags: unchecked; Components: relay

[Files]
Source: "{#PayloadDir}\rat.exe"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\avcodec-63.dll"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\avdevice-63.dll"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\avfilter-12.dll"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\avformat-63.dll"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\avutil-61.dll"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\swresample-7.dll"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\swscale-10.dll"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\README.txt"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
; relay линкует часть FFmpeg-библиотек — кладём их и в его компонент.
Source: "{#PayloadDir}\rat-relay.exe"; DestDir: "{app}"; Components: relay; Flags: ignoreversion
Source: "{#PayloadDir}\avcodec-63.dll"; DestDir: "{app}"; Components: relay; Flags: ignoreversion
Source: "{#PayloadDir}\avutil-61.dll"; DestDir: "{app}"; Components: relay; Flags: ignoreversion
Source: "{#PayloadDir}\swscale-10.dll"; DestDir: "{app}"; Components: relay; Flags: ignoreversion
Source: "{#PayloadDir}\swresample-7.dll"; DestDir: "{app}"; Components: relay; Flags: ignoreversion

[Icons]
Name: "{group}\Rat Next"; Filename: "{app}\rat.exe"; Components: rat
Name: "{group}\Rat Next relay"; Filename: "{app}\rat-relay.exe"; Components: relay
Name: "{group}\{cm:UninstallProgram,Rat Next}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Rat Next"; Filename: "{app}\rat.exe"; Tasks: desktopicon; Components: rat

[Run]
Filename: "{app}\rat.exe"; Parameters: "--install-autostart"; Tasks: autostart; Components: rat; Flags: runhidden; StatusMsg: "Включаю автозапуск…"
Filename: "{app}\rat-relay.exe"; Parameters: "--install-autostart"; Tasks: relay_autostart; Components: relay; Flags: runhidden; StatusMsg: "Включаю автозапуск relay…"
Filename: "{app}\rat.exe"; Description: "{cm:LaunchProgram,Rat Next}"; Components: rat; Flags: nowait postinstall skipifsilent
Filename: "{app}\rat-relay.exe"; Description: "Запустить relay-сервер"; Components: relay; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Без Components: файла нет — команда пропускается (skipifdoesntexist),
; поэтому логика верна для любой комбинации установленных компонентов.
Filename: "{app}\rat.exe"; Parameters: "--uninstall-service"; Flags: runhidden skipifdoesntexist; RunOnceId: "RemoveService"
Filename: "{app}\rat-relay.exe"; Parameters: "--uninstall-service"; Flags: runhidden skipifdoesntexist; RunOnceId: "RemoveRelayService"
Filename: "{app}\rat.exe"; Parameters: "--remove-autostart"; Flags: runhidden skipifdoesntexist; RunOnceId: "RemoveAutostart"
Filename: "{app}\rat-relay.exe"; Parameters: "--remove-autostart"; Flags: runhidden skipifdoesntexist; RunOnceId: "RemoveRelayAutostart"

[UninstallDelete]
Type: filesandordirs; Name: "{app}"