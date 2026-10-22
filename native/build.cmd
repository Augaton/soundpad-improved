@echo off
rem Builds the microphone filter with MinGW-w64 (MSYS2 UCRT64, g++ in PATH):
rem   bin\SoundpadImprovedApo.dll  the APO that audiodg.exe loads
rem   bin\spi-feed.exe             the bridge from the app to the APO
rem   bin\test_apo.exe             the end-to-end test (run: bin\test_apo.exe bin)
rem Everything is linked statically: audiodg.exe cannot find MinGW's DLLs.
setlocal
cd /d "%~dp0"
if not exist bin mkdir bin
set FLAGS=-O2 -std=c++17 -Wall -Wextra -static -static-libgcc -static-libstdc++
g++ %FLAGS% -fno-exceptions -fno-rtti -shared -o bin\SoundpadImprovedApo.dll apo.cpp apo.def -lole32 -luuid -ladvapi32 || exit /b 1
g++ %FLAGS% -fno-exceptions -fno-rtti -o bin\spi-feed.exe feed.cpp -ladvapi32 || exit /b 1
g++ %FLAGS% -o bin\test_apo.exe test_apo.cpp -lole32 -luuid -ladvapi32 -lwinmm || exit /b 1
echo OK
