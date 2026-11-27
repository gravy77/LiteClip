# Builds LiteClip in release mode and copies LiteClip.exe next to this script.
# Needs: rustup with the stable-x86_64-pc-windows-gnu toolchain
#        WinLibs MinGW (winget install BrechtSanders.WinLibs.POSIX.UCRT)

# Add WinLibs MinGW (installed by winget) to PATH if present.
$mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory -ErrorAction SilentlyContinue |
    Where-Object Name -like "*WinLibs*" | ForEach-Object { Join-Path $_.FullName "mingw64\bin" } |
    Where-Object { Test-Path $_ } | Select-Object -First 1
$env:PATH = "$env:USERPROFILE\.cargo\bin;$mingw;$env:PATH"

# Short build folder: some Windows build scripts fail on very long paths.
$env:CARGO_TARGET_DIR = Join-Path $env:USERPROFILE ".cache\liteclip-target"

# Keep local folder names (e.g. your Windows username) out of the compiled exe.
$env:RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~ --remap-path-prefix=$PSScriptRoot=liteclip"

Set-Location $PSScriptRoot
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Copy-Item (Join-Path $env:CARGO_TARGET_DIR "release\liteclip.exe") "$PSScriptRoot\LiteClip.exe" -Force
Write-Host "Built: $PSScriptRoot\LiteClip.exe"
