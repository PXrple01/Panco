@echo off
rem Doppelklick: misst Panco (RAM/CPU) und lässt das Fenster offen. Panco muss laufen.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0measure.ps1" -Voice
