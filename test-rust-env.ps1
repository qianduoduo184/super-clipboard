# Rust 环境测试脚本
# 用于验证 Rust 工具链和 MSVC 链接器是否正确安装

Write-Host "=== Rust 环境检查 ===" -ForegroundColor Cyan

# 1. 检查 Rust 版本
Write-Host "`n1. Rust 编译器版本:" -ForegroundColor Yellow
try {
    $rustVersion = rustc --version
    Write-Host "   ✓ $rustVersion" -ForegroundColor Green
} catch {
    Write-Host "   ✗ Rust 未安装或不在 PATH 中" -ForegroundColor Red
    exit 1
}

# 2. 检查 Cargo 版本
Write-Host "`n2. Cargo 版本:" -ForegroundColor Yellow
try {
    $cargoVersion = cargo --version
    Write-Host "   ✓ $cargoVersion" -ForegroundColor Green
} catch {
    Write-Host "   ✗ Cargo 未安装或不在 PATH 中" -ForegroundColor Red
    exit 1
}

# 3. 检查 MSVC 链接器
Write-Host "`n3. MSVC 链接器:" -ForegroundColor Yellow
$vsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vsWhere) {
    $vsPath = & $vsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($vsPath) {
        Write-Host "   ✓ Visual Studio Build Tools 已安装: $vsPath" -ForegroundColor Green

        # 查找 link.exe
        $vcToolsPath = Join-Path $vsPath "VC\Tools\MSVC"
        if (Test-Path $vcToolsPath) {
            $latestMsvc = Get-ChildItem $vcToolsPath | Sort-Object Name -Descending | Select-Object -First 1
            $linkExe = Join-Path $latestMsvc.FullName "bin\Hostx64\x64\link.exe"
            if (Test-Path $linkExe) {
                Write-Host "   ✓ link.exe 找到: $linkExe" -ForegroundColor Green
            } else {
                Write-Host "   ✗ link.exe 未找到" -ForegroundColor Red
            }
        }
    } else {
        Write-Host "   ✗ Visual Studio Build Tools 未找到" -ForegroundColor Red
        Write-Host "   提示: 需要安装 Visual Studio Build Tools 并包含 C++ 工具" -ForegroundColor Yellow
    }
} else {
    Write-Host "   ✗ vswhere.exe 未找到，无法检测 Visual Studio" -ForegroundColor Red
}

# 4. 测试编译简单的 Rust 项目
Write-Host "`n4. 测试编译 Rust 代码:" -ForegroundColor Yellow
$testDir = Join-Path $env:TEMP "rust-test-$(Get-Random)"
New-Item -ItemType Directory -Path $testDir -Force | Out-Null

Push-Location $testDir
try {
    # 创建简单的 Rust 项目
    cargo new --bin test-project 2>&1 | Out-Null
    Set-Location test-project

    # 尝试编译
    Write-Host "   正在编译测试项目..." -ForegroundColor Gray
    $buildOutput = cargo build 2>&1

    if ($LASTEXITCODE -eq 0) {
        Write-Host "   ✓ 编译成功！Rust 环境完全正常" -ForegroundColor Green

        # 运行程序
        $runOutput = cargo run --quiet 2>&1
        Write-Host "   ✓ 运行输出: $runOutput" -ForegroundColor Green
    } else {
        Write-Host "   ✗ 编译失败" -ForegroundColor Red
        Write-Host "错误信息:" -ForegroundColor Red
        Write-Host $buildOutput -ForegroundColor Gray
        Pop-Location
        Remove-Item -Recurse -Force $testDir
        exit 1
    }
} finally {
    Pop-Location
    Remove-Item -Recurse -Force $testDir
}

Write-Host "`n=== 环境检查完成 ===" -ForegroundColor Cyan
Write-Host "✓ Rust 工具链已就绪，可以编译 Tauri 项目" -ForegroundColor Green
