@echo off
chcp 65001 >nul
title Rat Next — Агент (LAN)
echo Запуск агента Rat Next (слушает все интерфейсы, порт 55140).
echo Если Windows спросит про брандмауэр — разрешите доступ для частных сетей.
echo.
"%~dp0..\target\release\rat-agent.exe" --listen 0.0.0.0:55140
echo.
echo Агент остановлен.
pause