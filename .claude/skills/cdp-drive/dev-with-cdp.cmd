@echo off
REM Launches the dev build with every WebView2 exposed on CDP port 9333, so Playwright's
REM connectOverCDP can drive the strip, flyout, settings and overlay pages.
REM A .cmd file, not an inline string: sv.ps1 -Cmd mis-tokenises `cmd /c "set X=Y&& ..."`.
set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333
cd /d "%~dp0..\..\.."
npm run tauri dev
