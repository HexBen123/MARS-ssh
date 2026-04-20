// Package service installs and uninstalls the relay/agent binaries as a
// platform-native service (systemd on Linux, Windows SCM on Windows).
// All operations are implemented by build-tagged files in this package.
package service

type Spec struct {
	Name        string // e.g. "mars-agent"
	DisplayName string // human-readable label (Windows service / unit description)
	Description string
	BinPath     string // absolute path to the binary
	ConfigPath  string // absolute path to the YAML config; used to generate the `ms` shortcut
	Args        []string
	// User/Group are honored on Linux; ignored on Windows (runs as LocalSystem).
	User  string
	Group string
}

// Status is the observable state of a named service on the host.
type Status struct {
	Installed bool   // registered with systemd / Windows SCM
	Running   bool   // currently active
	Enabled   bool   // auto-start at boot
	Detail    string // human-readable extra (e.g. "active (running) since ...")
}
