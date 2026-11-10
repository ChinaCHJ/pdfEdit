@echo off
chcp 65001 >nul
cd /d "%~dp0"

echo [1/2] 编译 release 版本...
cargo build --release
if errorlevel 1 (
    echo 编译失败。
    pause
    exit /b 1
)

echo [2/2] 复制到 dist...
if not exist dist mkdir dist
copy /y "target\release\pdf-editor.exe" "dist\pdf-editor.exe" >nul

echo.
echo   完成: %CD%\dist\pdf-editor.exe
echo.
echo   前端资源已经编进 exe，这一个文件拷到哪里都能直接双击运行。
echo   目标机器只需要装有 Edge（Windows 10/11 自带）或 Chrome，用来生成 PDF。
echo.
pause
