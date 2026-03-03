@echo off
setlocal
cd /d "%~dp0"

set CHART=%~1
set DIFF=%~2

set RUST_BACKTRACE=1
set CARGO_INCREMENTAL=0
set CARGO_BUILD_JOBS=1
if "%CHART%"=="" (
  cargo run --manifest-path mini_stepmania_rust\Cargo.toml --
) else if "%DIFF%"=="" (
  cargo run --manifest-path mini_stepmania_rust\Cargo.toml -- "%CHART%"
) else (
  cargo run --manifest-path mini_stepmania_rust\Cargo.toml -- "%CHART%" "%DIFF%"
)

if errorlevel 1 (
  echo.
  echo Run failed. See error above.
  pause
  exit /b 1
)
