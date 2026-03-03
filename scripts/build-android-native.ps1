param(
    [string]$TargetAbi = "arm64-v8a",
    [switch]$Release = $true
)

$ErrorActionPreference = "Stop"

if (-not $env:ANDROID_NDK_HOME -and $env:NDK_HOME) {
    $env:ANDROID_NDK_HOME = $env:NDK_HOME
}

if (-not $env:ANDROID_NDK_HOME) {
    Write-Error "Please set ANDROID_NDK_HOME or NDK_HOME."
    exit 1
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo not found in PATH."
    exit 1
}

$cargoNdk = Get-Command cargo-ndk -ErrorAction SilentlyContinue
if (-not $cargoNdk) {
    Write-Host "Installing cargo-ndk..."
    cargo install cargo-ndk --locked
}

$buildMode = if ($Release) { "--release" } else { "" }
$outDir = "target\android-jniLibs"

Write-Host "Building native library for $TargetAbi ..."
cargo ndk -t $TargetAbi -o $outDir build $buildMode

Write-Host "Done."
Write-Host "Output directory: $outDir\$TargetAbi"
