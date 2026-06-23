# 完整的自动化测试脚本
# 运行前端和后端的所有测试

param(
    [switch]$SkipFrontend,
    [switch]$SkipBackend,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"
$script:TestsFailed = $false

function Write-TestHeader {
    param([string]$Message)
    Write-Host "`n========================================" -ForegroundColor Cyan
    Write-Host " $Message" -ForegroundColor Cyan
    Write-Host "========================================`n" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "✓ $Message" -ForegroundColor Green
}

function Write-Failure {
    param([string]$Message)
    Write-Host "✗ $Message" -ForegroundColor Red
    $script:TestsFailed = $true
}

function Write-Info {
    param([string]$Message)
    Write-Host "ℹ $Message" -ForegroundColor Yellow
}

# 确保在项目根目录
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $scriptDir

Write-TestHeader "Super-Clipboard 自动化测试套件"

# 1. 前端测试
if (-not $SkipFrontend) {
    Write-TestHeader "前端测试"

    # 1.1 TypeScript 类型检查
    Write-Info "运行 TypeScript 类型检查..."
    try {
        $output = npm run typecheck 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "TypeScript 类型检查通过"
        } else {
            Write-Failure "TypeScript 类型检查失败"
            if ($Verbose) { Write-Host $output }
        }
    } catch {
        Write-Failure "TypeScript 类型检查异常: $_"
    }

    # 1.2 前端单元测试
    Write-Info "运行前端单元测试..."
    try {
        $output = npm test 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "前端测试通过"
            if ($Verbose) { Write-Host $output }
        } else {
            Write-Failure "前端测试失败"
            Write-Host $output
        }
    } catch {
        Write-Failure "前端测试异常: $_"
    }

    # 1.3 前端构建测试
    Write-Info "测试前端构建..."
    try {
        $output = npm run build 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "前端构建成功"
            # 检查构建产物
            if (Test-Path "dist") {
                Write-Success "构建产物已生成 (dist/)"
            } else {
                Write-Failure "构建产物目录不存在"
            }
        } else {
            Write-Failure "前端构建失败"
            if ($Verbose) { Write-Host $output }
        }
    } catch {
        Write-Failure "前端构建异常: $_"
    }
} else {
    Write-Info "跳过前端测试"
}

# 2. 后端测试
if (-not $SkipBackend) {
    Write-TestHeader "后端测试"

    # 2.1 检查 Rust 环境
    Write-Info "检查 Rust 工具链..."
    try {
        $rustVersion = rustc --version 2>&1
        $cargoVersion = cargo --version 2>&1
        Write-Success "Rust: $rustVersion"
        Write-Success "Cargo: $cargoVersion"
    } catch {
        Write-Failure "Rust 工具链未安装"
        $script:TestsFailed = $true
    }

    # 2.2 Cargo 测试
    Write-Info "运行 Rust 后端测试..."
    try {
        Push-Location src-tauri
        $output = cargo test 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "后端测试通过"
            if ($Verbose) { Write-Host $output }
        } else {
            Write-Failure "后端测试失败"
            Write-Host $output
        }
        Pop-Location
    } catch {
        Write-Failure "后端测试异常: $_"
        Pop-Location
    }

    # 2.3 Cargo 检查（不编译，只检查）
    Write-Info "运行 Cargo 检查..."
    try {
        Push-Location src-tauri
        $output = cargo check 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Cargo 检查通过"
        } else {
            Write-Failure "Cargo 检查失败"
            if ($Verbose) { Write-Host $output }
        }
        Pop-Location
    } catch {
        Write-Failure "Cargo 检查异常: $_"
        Pop-Location
    }
} else {
    Write-Info "跳过后端测试"
}

# 3. 测试总结
Write-TestHeader "测试总结"

if ($script:TestsFailed) {
    Write-Host "❌ 部分测试失败" -ForegroundColor Red
    exit 1
} else {
    Write-Host "✅ 所有测试通过！" -ForegroundColor Green
    exit 0
}
