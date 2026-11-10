@echo off
chcp 65001 >nul
cd /d "%~dp0"

if not exist "target\release\pdf-editor.exe" (
    echo 首次启动需要编译，请稍候...
    cargo build --release
    if errorlevel 1 (
        echo 编译失败，请检查是否已安装 Rust 工具链。
        pause
        exit /b 1
    )
)

rem 浏览器由程序自己打开，这里不用再起一次
target\release\pdf-editor.exe
pause
