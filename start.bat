@echo off
echo Starting Preference Database...

:: Kill any existing processes
taskkill /F /IM preference-database.exe >nul 2>&1
taskkill /F /IM node.exe >nul 2>&1

:: Wait for processes to terminate
timeout /t 2 >nul

:: Start the development server
npm run tauri dev
