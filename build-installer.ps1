# Build Windows Installer for SR-Player
# PowerShell script to automate the build process

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  SR-Player Windows Installer Builder" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# Check if cargo-wix is installed
Write-Host "Checking for cargo-wix..." -ForegroundColor Yellow
$cargoWix = Get-Command cargo-wix -ErrorAction SilentlyContinue
if (-not $cargoWix) {
    Write-Host "cargo-wix not found. Installing..." -ForegroundColor Red
    cargo install cargo-wix
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Failed to install cargo-wix. Please install manually." -ForegroundColor Red
        exit 1
    }
}
Write-Host "✓ cargo-wix is installed" -ForegroundColor Green
Write-Host ""

# Check for WiX Toolset
Write-Host "Checking for WiX Toolset..." -ForegroundColor Yellow
$candle = Get-Command candle -ErrorAction SilentlyContinue
if (-not $candle) {
    Write-Host "WiX Toolset not found in PATH!" -ForegroundColor Red
    Write-Host "Please install WiX Toolset 3.11 from:" -ForegroundColor Yellow
    Write-Host "https://wixtoolset.org/releases/" -ForegroundColor Cyan
    exit 1
}
Write-Host "✓ WiX Toolset is installed" -ForegroundColor Green
Write-Host ""

# Build release executable
Write-Host "Building release executable..." -ForegroundColor Yellow
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}
Write-Host "✓ Release build complete" -ForegroundColor Green
Write-Host ""

# Check for icon file
if (-not (Test-Path "wix\Product.ico")) {
    Write-Host "Warning: wix\Product.ico not found" -ForegroundColor Yellow
    Write-Host "The installer will be built without a custom icon." -ForegroundColor Yellow
    Write-Host "To add an icon, place a .ico file at wix\Product.ico" -ForegroundColor Yellow
    Write-Host ""
}

# Build installer
Write-Host "Creating MSI installer..." -ForegroundColor Yellow
cargo wix --nocapture
if ($LASTEXITCODE -ne 0) {
    Write-Host "Installer creation failed!" -ForegroundColor Red
    exit 1
}
Write-Host "✓ Installer created successfully" -ForegroundColor Green
Write-Host ""

# Find the installer file
$installer = Get-ChildItem -Path "target\wix" -Filter "*.msi" | Select-Object -First 1
if ($installer) {
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  SUCCESS!" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Installer created at:" -ForegroundColor Yellow
    Write-Host $installer.FullName -ForegroundColor Cyan
    Write-Host ""
    Write-Host "File size:" $installer.Length "bytes" -ForegroundColor Gray
    Write-Host ""
    Write-Host "To test the installer:" -ForegroundColor Yellow
    Write-Host "  1. Right-click the .msi file" -ForegroundColor White
    Write-Host "  2. Select 'Install'" -ForegroundColor White
    Write-Host ""
    Write-Host "Next steps:" -ForegroundColor Yellow
    Write-Host "  - Sign the installer to remove security warnings" -ForegroundColor White
    Write-Host "    See CODE_SIGNING.md for instructions" -ForegroundColor Gray
    Write-Host ""
} else {
    Write-Host "Error: Installer file not found in target\wix\" -ForegroundColor Red
    exit 1
}
