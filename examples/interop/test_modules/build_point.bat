@echo off
rem Build point.dll from point.cpp using MSVC.
rem Run this from a "Developer Command Prompt for VS" (or after calling vcvarsall.bat).
rem Exports are declared in point.def (no __declspec needed in the source).

echo Building point.dll ...
cl /LD /EHsc /nologo point.cpp /link /DEF:point.def
if %ERRORLEVEL% EQU 0 (
    echo Done: point.dll
) else (
    echo Build failed.
    exit /b 1
)
