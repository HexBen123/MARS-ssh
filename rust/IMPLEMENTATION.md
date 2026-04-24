# Rust Rewrite Implementation Plan

Goal: migrate MARS to Rust while preserving the existing Go behavior, command
surface, config files, wire protocol, and deployment flow.

Scope:
- Produce Rust `agent` and `relay` binaries from the `rust/` crate.
- Keep existing YAML and JSON state formats compatible.
- Keep the length-prefixed JSON control protocol compatible.
- Keep TLS fingerprint pinning semantics compatible.
- Keep yamux stream bridging semantics compatible.
- Preserve `run`, `ms`, `install`, and `uninstall` command names.

Phases:

1. Baseline prototype
   - Keep the already verified Rust agent protocol prototype.
   - Commit after tests and release build pass.

2. Shared core modules
   - Split `src/lib.rs` into focused modules for config, protocol, TLS, state,
     port pool, logging, public IP discovery, service management, and menu.
   - Add tests for config validation, state persistence, port allocation, and
     protocol framing.
   - Commit after `cargo test` and `cargo build --release`.

3. Rust relay
   - Add `src/bin/relay.rs`.
   - Implement first-run relay wizard, config loading, TLS listener, agent
     authentication, sticky port allocation, public listener per agent, and
     stream bridging.
   - Verify with the existing Go agent and the Rust agent.
   - Commit after compatibility testing.

4. Rust agent completeness
   - Add first-run and edit-config wizard.
   - Write `agent-info.txt` after successful registration.
   - Preserve reconnect behavior and local bridge behavior.
   - Commit after compatibility testing against Rust relay and Go relay.

5. Menu and service management
   - Port `ms` menu behavior for both roles.
   - Port Windows SCM and Linux systemd install/uninstall/status/start/stop
     behavior as closely as practical.
   - Preserve `ms` shortcut generation.
   - Commit after command-surface checks.

6. Build and documentation
   - Add a Rust build script for Windows-hosted cross-target builds if the
     local toolchain supports the targets.
   - Document current feature parity and any intentional platform limitations.
   - Record release binary sizes.
   - Commit after final verification.
