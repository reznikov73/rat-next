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
  { До 10 секунд: служба может завершать помощника «по-хорошему» (вежливо). }
  for I := 1 to 40 do
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

{ --- Вежливое завершение приложений перед обновлением -------------------- }
function OpenEventW(desiredAccess: Cardinal; inheritHandle: Cardinal; name: String): Integer;
  external 'OpenEventW@kernel32.dll stdcall';
function SetEvent(hEvent: Integer): Boolean;
  external 'SetEvent@kernel32.dll stdcall';
function CloseHandle(hObject: Integer): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';

function AppStillRunning(ExeName: String): Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(
    ExpandConstant('{cmd}'),
    '/c tasklist /FI "IMAGENAME eq ' + ExeName + '" /NH | findstr /I "' + ExeName + '" >nul',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

{ Просим работающий экземпляр завершиться самому: он штатно уберёт значок
  трея (иначе в панели остаётся «мёртвый» значок). Имя события совпадает с
  тем, что создаёт приложение; старые версии события не слушают — просто
  ничего не произойдёт. }
procedure RequestAppQuit(ExeName: String; Tag: String);
var
  H: Integer;
  S: String;
  I: Integer;
begin
  S := LowerCase(ExpandConstant('{app}\') + ExeName);
  for I := 1 to Length(S) do
    if S[I] = '\' then
      S[I] := '_';
  H := OpenEventW($0002, 0, 'rat-next-quit-' + Tag + '-' + S); { EVENT_MODIFY_STATE }
  if H <> 0 then
  begin
    SetEvent(H);
    CloseHandle(H);
  end;
  { Ждём выхода приложения до ~5 секунд. }
  for I := 1 to 20 do
  begin
    if not AppStillRunning(ExeName) then
      Break;
    Sleep(250);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssInstall then
  begin
    StopOldServices;
    RequestAppQuit('rat.exe', 'app');
    RequestAppQuit('rat-relay.exe', 'relay');
  end;
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
; Службы включаются здесь одной галочкой (по умолчанию включены); позже их
; можно менять в настройках самого приложения (Rat Next / relay).
Name: "service"; Description: "Включить службу Rat Next (рекомендуется: доступ с загрузки ПК)"; GroupDescription: "Дополнительные параметры:"; Components: rat
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "Дополнительные параметры:"; Flags: unchecked; Components: rat
Name: "relay_service"; Description: "Включить службу relay (рекомендуется для серверной машины)"; GroupDescription: "Дополнительные параметры:"; Components: relay

[Files]
Source: "{#PayloadDir}\rat.exe"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
Source: "{#PayloadDir}\rat-elevate.exe"; DestDir: "{app}"; Components: rat; Flags: ignoreversion
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
Name: "{group}\Rat Next"; Filename: "{app}\rat-elevate.exe"; IconFilename: "{app}\rat.exe"; Components: rat
Name: "{group}\Rat Next relay"; Filename: "{app}\rat-relay.exe"; Components: relay
Name: "{group}\{cm:UninstallProgram,Rat Next}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Rat Next"; Filename: "{app}\rat-elevate.exe"; IconFilename: "{app}\rat.exe"; Tasks: desktopicon; Components: rat

[Run]
Filename: "{app}\rat.exe"; Parameters: "--install-service"; Tasks: service; Components: rat; Flags: runhidden; StatusMsg: "Включаю службу Rat Next…"
Filename: "{app}\rat-relay.exe"; Parameters: "--install-service"; Tasks: relay_service; Components: relay; Flags: runhidden; StatusMsg: "Включаю службу relay…"
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