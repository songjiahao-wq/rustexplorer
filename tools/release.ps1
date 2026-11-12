# RustExplorer 发版脚本：编译 -> 注入图标 -> 复制到 releases
# 用法: powershell -File tools/release.ps1            (默认用 assets/icon.ico)
#       powershell -File tools/release.ps1 -Variant b (先按方案 b 重生成图标)
param(
    [string]$Variant = "",
    [string]$Out = "D:\AI_project\workbuddy\releases\rustexplorer.exe"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$py = "C:\Users\Administrator\AppData\Local\Programs\Python\Python312\python.exe"

Push-Location $root
try {
    if ($Variant -ne "") {
        Write-Host "== 重新生成图标（方案 $Variant）" -ForegroundColor Cyan
        & $py tools\gen_icon.py $Variant
    }
    Write-Host "== cargo build --release" -ForegroundColor Cyan
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "编译失败" }

    $exe = Join-Path $root "target\release\rustexplorer.exe"
    Write-Host "== 注入图标资源" -ForegroundColor Cyan
    & $py tools\patch_icon.py $exe
    if ($LASTEXITCODE -ne 0) { throw "图标注入失败" }

    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null
    Copy-Item $exe $Out -Force
    $f = Get-Item $Out
    Write-Host ("== OK  {0}  {1:N0} bytes  {2}" -f $f.Name, $f.Length, $f.LastWriteTime) -ForegroundColor Green
} finally {
    Pop-Location
}
