param(
    [string]$Configuration = "release"
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function Write-SmokeLog {
    param([string]$Message)
    Write-Output "[smoke] $Message"
}

function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse('127.0.0.1'), 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $port
}

function Wait-Port {
    param([int]$Port, [int]$TimeoutMs)

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $client = [System.Net.Sockets.TcpClient]::new()
            $pending = $client.BeginConnect('127.0.0.1', $Port, $null, $null)
            if ($pending.AsyncWaitHandle.WaitOne(150)) {
                $client.EndConnect($pending)
                $client.Close()
                return $true
            }
            $client.Close()
        } catch {
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Wait-AgentRegistered {
    param([string]$StatePath, [int]$TimeoutMs)

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $StatePath) {
            $text = Get-Content -LiteralPath $StatePath -Raw -ErrorAction SilentlyContinue
            if ($text -like '*smoke-agent*') {
                return $true
            }
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$targetDir = Join-Path $root 'rust\target'
$profileDir = if ($Configuration -eq 'debug') { 'debug' } else { 'release' }
$relayExe = Join-Path $targetDir "$profileDir\relay.exe"
$agentExe = Join-Path $targetDir "$profileDir\agent.exe"
$stamp = "smoke-$PID-$(Get-Date -Format 'yyyyMMddHHmmss')"
$workDir = Join-Path $targetDir $stamp
New-Item -ItemType Directory -Force -Path $workDir | Out-Null

$cert = Join-Path $root 'rust\tests\fixtures\relay_cert.pem'
$key = Join-Path $root 'rust\tests\fixtures\relay_key.pem'
$relayCfg = Join-Path $workDir 'relay.yaml'
$agentCfg = Join-Path $workDir 'agent.yaml'
$state = Join-Path $workDir 'state.json'
$controlPort = Get-FreePort
$publicPort = Get-FreePort
$localPort = Get-FreePort
$fingerprint = 'sha256:74b49e8e666e83cacb4c8e19cba2d12045ef49e25e6ab6e324d628e57ccf81df'

$relayYaml = @"
listen: 127.0.0.1:$controlPort
public_host: 127.0.0.1
token: secret
tls:
  cert: $cert
  key: $key
port_range:
  min: $publicPort
  max: $publicPort
state_file: $state
"@

$agentYaml = @"
relay: 127.0.0.1:$controlPort
server_name: 127.0.0.1
fingerprint: $fingerprint
token: secret
agent_id: smoke-agent
local_addr: 127.0.0.1:$localPort
"@

[System.IO.File]::WriteAllText($relayCfg, $relayYaml, [System.Text.Encoding]::UTF8)
[System.IO.File]::WriteAllText($agentCfg, $agentYaml, [System.Text.Encoding]::UTF8)

Write-SmokeLog "workdir=$workDir"
Write-SmokeLog "ports control=$controlPort public=$publicPort local=$localPort"

$relay = $null
$agent = $null
$localServer = $null

try {
    $localServer = Start-Job -ScriptBlock {
        param($Port)
        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Parse('127.0.0.1'), $Port)
        $listener.Start()
        try {
            $client = $listener.AcceptTcpClient()
            $client.ReceiveTimeout = 5000
            $client.SendTimeout = 5000
            try {
                $stream = $client.GetStream()
                $buffer = New-Object byte[] 1024
                $count = $stream.Read($buffer, 0, $buffer.Length)
                $text = [System.Text.Encoding]::UTF8.GetString($buffer, 0, $count)
                $reply = [System.Text.Encoding]::UTF8.GetBytes("pong:$text")
                $stream.Write($reply, 0, $reply.Length)
            } finally {
                $client.Close()
            }
        } finally {
            $listener.Stop()
        }
    } -ArgumentList $localPort

    Start-Sleep -Milliseconds 300
    Write-SmokeLog 'starting relay'
    $relay = Start-Process -FilePath $relayExe -ArgumentList @('run', '-config', $relayCfg) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $workDir 'relay.out') -RedirectStandardError (Join-Path $workDir 'relay.err')
    if (-not (Wait-Port $controlPort 5000)) {
        throw "relay control port $controlPort did not open"
    }

    Write-SmokeLog 'starting agent'
    $agent = Start-Process -FilePath $agentExe -ArgumentList @('run', '-config', $agentCfg) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $workDir 'agent.out') -RedirectStandardError (Join-Path $workDir 'agent.err')
    if (-not (Wait-AgentRegistered $state 8000)) {
        throw "agent did not register in $state"
    }

    Write-SmokeLog 'agent registered; sending bridge payload'
    $client = [System.Net.Sockets.TcpClient]::new('127.0.0.1', $publicPort)
    $client.ReceiveTimeout = 5000
    $client.SendTimeout = 5000
    try {
        $stream = $client.GetStream()
        $payload = [System.Text.Encoding]::UTF8.GetBytes('ping')
        $stream.Write($payload, 0, $payload.Length)
        $buffer = New-Object byte[] 1024
        $count = $stream.Read($buffer, 0, $buffer.Length)
        $reply = [System.Text.Encoding]::UTF8.GetString($buffer, 0, $count)
        Write-SmokeLog "reply=$reply"
        if ($reply -ne 'pong:ping') {
            throw "unexpected bridge reply: $reply"
        }
    } finally {
        $client.Close()
    }

    Receive-Job -Job $localServer -Wait | Out-Null
    Write-SmokeLog 'result=ok'
} finally {
    if ($agent -and -not $agent.HasExited) {
        Stop-Process -Id $agent.Id -Force
    }
    if ($relay -and -not $relay.HasExited) {
        Stop-Process -Id $relay.Id -Force
    }
    if ($localServer) {
        Stop-Job -Job $localServer -ErrorAction SilentlyContinue
        Receive-Job -Job $localServer -ErrorAction SilentlyContinue | Out-Null
    }
}
