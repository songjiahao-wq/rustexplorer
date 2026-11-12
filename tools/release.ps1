# RustExplorer 发版脚本：编译 -> 注入图标 -> 复制到 releases ->（可选）发 GitHub Release
#
# 用法:
#   powershell -File tools\release.ps1                          # 只编译 + 注入图标 + 复制
#   powershell -File tools\release.ps1 -Variant b                # 先按方案 b 重生成图标
#   powershell -File tools\release.ps1 -Release                  # 另外建 tag v{版本号} 并发 Release
#   powershell -File tools\release.ps1 -Release -Notes a.md      # 用自定义发布说明
#
# 注意:
#   - GitHub 走用户本地代理（默认 20172，带分流规则那个端口）；工作区注入的透明代理对 github.com 会 502
#   - 中文一律走 UTF-8 文件传参，不要当命令行参数传给 git/gh，否则会被按 GBK 转坏
param(
    [string]$Variant = "",
    [string]$Out = "D:\AI_project\workbuddy\releases\rustexplorer.exe",
    [switch]$Release,
    [string]$Tag = "",
    [string]$Notes = "",
    [string]$Proxy = "http://127.0.0.1:20172"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$py = "C:\Users\Administrator\AppData\Local\Programs\Python\Python312\python.exe"

# 让 PowerShell 按 UTF-8 解码外部程序输出（否则 git log 里的中文会变乱码）
$prevOut = [Console]::OutputEncoding
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$env:HTTPS_PROXY = $Proxy
$env:HTTP_PROXY = $Proxy

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
    try {
        Copy-Item $exe $Out -Force
        $f = Get-Item $Out
        Write-Host ("== OK  {0}  {1:N0} bytes  {2}" -f $f.Name, $f.Length, $f.LastWriteTime) -ForegroundColor Green
    } catch {
        # 常见原因：旧的 exe 正在运行，句柄未释放。产物本身是好的，不阻断发版。
        Write-Host "!! 复制到 $Out 失败（可能该 exe 正在运行），已跳过；后续使用 target\release 下的产物" -ForegroundColor Yellow
    }

    if (-not $Release) { return }

    # ---- 版本号：默认取 Cargo.toml
    if ($Tag -eq "") {
        $ver = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
        $Tag = "v$ver"
    }

    # ---- 附件：用带版本号的名字，避开被占用的主产物
    $assetDir = Split-Path -Parent $Out
    $asset = Join-Path $assetDir ("rustexplorer-{0}.exe" -f $Tag)
    Copy-Item $exe $asset -Force
    $af = Get-Item $asset
    Write-Host ("== 附件  {0}  {1:N0} bytes" -f $af.Name, $af.Length) -ForegroundColor Cyan

    # ---- 发布说明：没指定就用自动生成的（版本 + 提交列表）
    $tmpNotes = Join-Path $env:TEMP "rustexplorer_release_$Tag.md"
    if ($Notes -ne "") {
        Copy-Item $Notes $tmpNotes -Force
    } else {
        $lines = @("## RustExplorer $Tag", "", "Rust + egui 写的 Windows 文件管理器，单 exe、免安装。", "", "### 本次提交", "")
        $lines += (git log --oneline -8 --no-decorate) | ForEach-Object { "- $_" }
        $lines += @("", "### 运行要求", "", "Windows 10 / 11 (x64)，无需安装任何运行时，双击即用。")
        $lines -join "`n" | Out-File -FilePath $tmpNotes -Encoding utf8NoBOM
    }

    # ---- tag：已存在就只推，不存在才建（说明走 UTF-8 文件）
    git rev-parse -q --verify "refs/tags/$Tag" 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host "== 创建 tag $Tag" -ForegroundColor Cyan
        git tag -a $Tag -F $tmpNotes
        if ($LASTEXITCODE -ne 0) { throw "建 tag 失败" }
    } else {
        Write-Host "== tag $Tag 已存在，跳过创建" -ForegroundColor Yellow
    }
    git push origin $Tag
    if ($LASTEXITCODE -ne 0) { throw "推送 tag 失败（检查代理端口 $Proxy）" }

    # ---- Release：已存在就覆盖上传附件
    gh release view $Tag --json tagName 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "== Release $Tag 已存在，覆盖上传附件" -ForegroundColor Yellow
        gh release upload $Tag $asset --clobber
    } else {
        Write-Host "== 创建 Release $Tag" -ForegroundColor Cyan
        gh release create $Tag $asset --title "RustExplorer $Tag" --notes-file $tmpNotes
    }
    if ($LASTEXITCODE -ne 0) { throw "创建 Release 失败" }

    Write-Host ("== Release 完成  https://github.com/songjiahao-wq/rustexplorer/releases/tag/{0}" -f $Tag) -ForegroundColor Green
} finally {
    [Console]::OutputEncoding = $prevOut
    Pop-Location
}
