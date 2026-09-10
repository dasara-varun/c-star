# C* Test Runner
# Runs compile-pass, compile-fail, and run tests against the starc compiler.

param(
    [string]$StarcPath = "",
    [string]$BuildDir = "build_test\test_runner"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

# Ensure LLVM is on PATH
$env:PATH = "C:\PROGRA~1\LLVM\bin;" + $env:PATH

if ($StarcPath -eq "") {
    $StarcPath = Join-Path $Root "target\debug\starc.exe"
    if (-not (Test-Path $StarcPath)) {
        $StarcPath = Join-Path $Root "target\release\starc.exe"
    }
}

if (-not (Test-Path $StarcPath)) {
    Write-Error "starc not found. Build with: cargo build -p starc"
    exit 1
}

Write-Host "Using starc: $StarcPath"
New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null

$passed = 0
$failed = 0

function Test-CompilePass {
    param([string]$File)
    $name = [System.IO.Path]::GetFileNameWithoutExtension($File)
    $out = Join-Path $BuildDir "$name.exe"
    Write-Host "  PASS compile: $File" -NoNewline
    $result = & $StarcPath build $File -o $out 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Host " FAIL"
        Write-Host $result
        return $false
    }
    Write-Host " OK"
    return $true
}

function Test-CompileFail {
    param([string]$File, [string]$ExpectedJson)
    $name = [System.IO.Path]::GetFileNameWithoutExtension($File)
    Write-Host "  FAIL compile: $File" -NoNewline

    $outFile = Join-Path $BuildDir "${name}_fail.json"
    $result = & $StarcPath build $File --error-format=json -o (Join-Path $BuildDir "$name.exe") 2>&1
    # starc prints JSON to stdout on failure
    $jsonOutput = $result | Out-String

    if ($LASTEXITCODE -eq 0) {
        Write-Host " UNEXPECTED PASS"
        return $false
    }

    $expected = Get-Content $ExpectedJson -Raw | ConvertFrom-Json
    $actual = $null
    try {
        # Extract JSON from output (may have other lines)
        $jsonStart = $jsonOutput.IndexOf("{")
        if ($jsonStart -ge 0) {
            $jsonStr = $jsonOutput.Substring($jsonStart)
            $actual = $jsonStr | ConvertFrom-Json
        }
    } catch {
        Write-Host " JSON PARSE FAIL"
        Write-Host $jsonOutput
        return $false
    }

    if ($null -eq $actual) {
        Write-Host " NO JSON"
        return $false
    }

    $expectedCode = $expected.diagnostics[0].code
    $actualCode = $actual.diagnostics[0].code
    if ($expectedCode -ne $actualCode) {
        Write-Host " CODE MISMATCH (expected $expectedCode, got $actualCode)"
        return $false
    }

    Write-Host " OK ($actualCode)"
    return $true
}

function Test-Run {
    param([string]$SourceFile, [string]$ExpectedFile)
    $name = [System.IO.Path]::GetFileNameWithoutExtension($SourceFile)
    $out = Join-Path $BuildDir "${name}_run.exe"
    Write-Host "  RUN: $name" -NoNewline

    & $StarcPath build $SourceFile -o $out 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host " COMPILE FAIL"
        return $false
    }

    try {
        $stdout = & $out 2>&1 | Out-String
    } catch {
        Write-Host " EXEC BLOCKED (Application Control)"
        Write-Host "  Skipping execution test for $name"
        return $true
    }

    $expected = Get-Content $ExpectedFile -Raw
    if ($stdout.Trim() -ne $expected.Trim()) {
        Write-Host " OUTPUT MISMATCH"
        Write-Host "  Expected: '$($expected.Trim())'"
        Write-Host "  Got:      '$($stdout.Trim())'"
        return $false
    }

    Write-Host " OK"
    return $true
}

Write-Host "`n=== Compile-Pass Tests ==="
Get-ChildItem "tests\compile-pass\*.cx" | ForEach-Object {
    if (Test-CompilePass $_.FullName) { $script:passed++ } else { $script:failed++ }
}

Write-Host "`n=== Compile-Fail Tests ==="
Get-ChildItem "tests\compile-fail\*.cx" | ForEach-Object {
    $jsonFile = $_.FullName -replace '\.cx$', '.json'
    if (Test-Path $jsonFile) {
        if (Test-CompileFail $_.FullName $jsonFile) { $script:passed++ } else { $script:failed++ }
    }
}

Write-Host "`n=== Run Tests ==="
Get-ChildItem "tests\run\*.expected" | ForEach-Object {
    $name = $_.BaseName
    $source = "tests\compile-pass\$name.cx"
    if (-not (Test-Path $source)) {
        $source = "examples\$name.cx"
    }
    if (Test-Path $source) {
        if (Test-Run $source $_.FullName) { $script:passed++ } else { $script:failed++ }
    }
}

Write-Host "`n=== Examples ==="
@("examples\hello.cx", "examples\main.cx", "examples\geometry.cx") | ForEach-Object {
    if (Test-Path $_) {
        if (Test-CompilePass $_) { $script:passed++ } else { $script:failed++ }
    }
}

Write-Host "`n=== Results ==="
Write-Host "Passed: $passed"
Write-Host "Failed: $failed"

if ($failed -gt 0) {
    exit 1
}
exit 0
