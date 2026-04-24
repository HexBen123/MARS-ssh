param()

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$outDir = Join-Path $root 'bin-rust'
$cargoHome = Join-Path $env:USERPROFILE '.cargo'

function ConvertTo-WslPath {
    param([string]$Path)

    $full = (Resolve-Path $Path).Path
    if ($full -notmatch '^([A-Za-z]):\\(.*)$') {
        throw "Only drive-letter paths can be converted to WSL paths: $full"
    }
    $drive = $Matches[1].ToLowerInvariant()
    $rest = $Matches[2] -replace '\\', '/'
    "/mnt/$drive/$rest"
}

function Quote-Sh {
    param([string]$Value)

    "'" + ($Value -replace "'", "'""'""'") + "'"
}

Push-Location $root
try {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null

    $env:CARGO_INCREMENTAL = '0'
    cargo build --release --bins
    Copy-Item -Force -Path 'target\release\agent.exe' -Destination (Join-Path $outDir 'agent.exe')
    Copy-Item -Force -Path 'target\release\relay.exe' -Destination (Join-Path $outDir 'relay.exe')

    cargo fetch --target x86_64-unknown-linux-gnu

    $wslRoot = ConvertTo-WslPath $root
    $wslCargoHome = ConvertTo-WslPath $cargoHome
    $linuxBuild = "cd $(Quote-Sh $wslRoot) && CARGO_HOME=$(Quote-Sh $wslCargoHome) CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=target/linux-release cargo build --release --bins"
    wsl sh -lc $linuxBuild

    Copy-Item -Force -Path 'target\linux-release\release\agent' -Destination (Join-Path $outDir 'agent-linux-amd64')
    Copy-Item -Force -Path 'target\linux-release\release\relay' -Destination (Join-Path $outDir 'relay-linux-amd64')

    $published = @(
        (Join-Path $outDir 'agent.exe')
        (Join-Path $outDir 'relay.exe')
        (Join-Path $outDir 'agent-linux-amd64')
        (Join-Path $outDir 'relay-linux-amd64')
    )
    Get-Item -Path $published |
        Select-Object Name, Length, @{ Name = 'MiB'; Expression = { [math]::Round($_.Length / 1MB, 3) } }
} finally {
    Pop-Location
}
