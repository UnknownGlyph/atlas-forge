@echo off
rem Runs cargo, inside an MSVC developer environment when one is given.
rem rustc finds a registered Visual Studio (or its Build Tools) by itself;
rem for one it cannot find, give the path of vcvars64.bat in VCVARS64, or
rem set it in tools\local.cmd (ignored by git), for example:
rem   set "VCVARS64=C:\VS\VC\Auxiliary\Build\vcvars64.bat"
if exist "%~dp0local.cmd" call "%~dp0local.cmd"
if not "%VCVARS64%"=="" call "%VCVARS64%" >nul || exit /b 1
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cargo %*
