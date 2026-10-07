# 一键打包：编译发布版，再用 Inno Setup 生成安装包。
$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

function Fail($message) {
    Write-Host $message -ForegroundColor Red
    exit 1
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Fail "找不到 cargo。请先安装 Rust，并确认它在 PATH 里。"
}

$iscc = @(
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $iscc) {
    Fail "找不到 Inno Setup 6。请安装后再打包。"
}

$running = Get-Process desk -ErrorAction SilentlyContinue
if ($running) {
    Write-Host "正在关闭已运行的案头，以便覆盖程序文件..."
    $running | Stop-Process -Force
    Start-Sleep -Milliseconds 400
}

Write-Host "正在编译发布版..."
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Fail "编译失败。"
}

Write-Host "正在生成安装包..."
& $iscc (Join-Path $PSScriptRoot "desk.iss")
if ($LASTEXITCODE -ne 0) {
    Fail "打包失败。"
}

$setup = Join-Path $PSScriptRoot "target\installer\desk-setup-0.1.0.exe"
if (-not (Test-Path -LiteralPath $setup)) {
    Fail "打包结束，但没有找到安装包。"
}
Write-Host ""
Write-Host "安装包已生成：" -ForegroundColor Green
Write-Host $setup
