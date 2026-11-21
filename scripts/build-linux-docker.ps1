# Build Linux .deb and .tar.gz packages locally via Docker
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

Write-Host "Building Linux x86_64 packages (.deb & .tar.gz) inside Docker (Debian Bookworm)..." -ForegroundColor Cyan
docker build -f Dockerfile.linux -t ytm-native-linux .

New-Item -ItemType Directory -Force -Path "dist" | Out-Null
docker rm -f ytm-linux-extract 2>$null | Out-Null
docker create --name ytm-linux-extract ytm-native-linux | Out-Null
docker cp ytm-linux-extract:/out/ytm-native_1.0.0-beta_amd64.deb ./dist/
docker cp ytm-linux-extract:/out/YTM-Native-1.0.0-beta-linux-x64.tar.gz ./dist/
docker rm -f ytm-linux-extract | Out-Null

Write-Host "Linux packages extracted to ./dist/:" -ForegroundColor Green
Get-Item "dist\ytm-native_*.deb", "dist\YTM-Native-*-linux-x64.tar.gz" | Select-Object FullName, @{N="SizeMB";E={[math]::Round($_.Length/1MB,2)}}
