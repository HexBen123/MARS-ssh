# Rust Rewrite Status

Goal: migrate MARS to Rust while preserving the existing Go behavior, command
surface, config files, wire protocol, and deployment flow.

Current status: feature parity is implemented in the `rust/` crate.

## Delivered Scope

- Produces `agent.exe` and `relay.exe` from explicit Cargo bin targets.
- Preserves the YAML config and JSON state formats used by the Go version.
- Preserves the 4-byte length-prefixed JSON control protocol.
- Preserves TLS certificate fingerprint pinning and self-signed relay cert generation.
- Preserves yamux-based stream multiplexing and TCP bridge behavior.
- Preserves `run`, `ms`, `install`, and `uninstall` command names for both roles.
- Preserves first-run and edit-config wizards for both agent and relay.
- Preserves `agent-info.txt` generation after a successful agent registration.
- Preserves public IPv4 auto-discovery in the relay wizard.
- Preserves log fan-out to stderr plus config-directory log files, with 10 MiB rotation.
- Preserves service management behavior:
  - Linux: systemd install, uninstall, status, start, stop, restart, enable, disable.
  - Windows: SCM install, uninstall, status, start, stop, restart, enable, disable.
  - Windows install also configures service failure restart actions.
  - `ms` shortcut generation is retained on Linux and Windows.

## Verification

Run from `rust/`:

```powershell
$env:CARGO_INCREMENTAL='0'; cargo test
$env:CARGO_INCREMENTAL='0'; cargo build --release --bins
```

Or from the repository root:

```powershell
.\rust\scripts\build-release.ps1
.\rust\scripts\smoke-bridge.ps1 -Configuration release
```

The smoke script starts the release `relay.exe` and `agent.exe`, registers the
agent through the relay, sends `ping` through the assigned public port, and
expects `pong:ping` from a local TCP echo service behind the agent.

Latest verified Windows release sizes on this workstation:

| Binary | Size |
| --- | ---: |
| `agent.exe` | 764416 bytes, 0.73 MiB |
| `relay.exe` | 1632768 bytes, 1.56 MiB |

The Go implementation remains in the repository as the baseline/reference while
the Rust branch is being validated.
