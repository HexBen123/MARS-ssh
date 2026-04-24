param()

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    $env:CARGO_INCREMENTAL = '0'
    cargo build --release --bins

    Get-Item -Path 'target\release\agent.exe', 'target\release\relay.exe' |
        Select-Object Name, Length, @{ Name = 'MiB'; Expression = { [math]::Round($_.Length / 1MB, 3) } }
} finally {
    Pop-Location
}
