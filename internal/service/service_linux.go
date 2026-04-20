//go:build linux

package service

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const unitDir = "/etc/systemd/system"

// msShortcut is the global shell wrapper we install so users can type `ms`
// from anywhere to open the management menu. Only one role (relay or agent)
// can own it at a time — the wrapper carries an `mars-shortcut-for=<name>`
// marker so Uninstall only removes it when it belongs to that service.
const msShortcut = "/usr/local/bin/ms"

func Install(s Spec) error {
	unitPath := filepath.Join(unitDir, s.Name+".service")

	var b strings.Builder
	fmt.Fprintf(&b, "[Unit]\nDescription=%s\nAfter=network-online.target\nWants=network-online.target\n\n", nonempty(s.Description, s.DisplayName, s.Name))
	b.WriteString("[Service]\nType=simple\n")
	fmt.Fprintf(&b, "ExecStart=%q", s.BinPath)
	for _, a := range s.Args {
		fmt.Fprintf(&b, " %q", a)
	}
	b.WriteString("\nRestart=always\nRestartSec=3\n")
	if s.User != "" {
		fmt.Fprintf(&b, "User=%s\n", s.User)
	}
	if s.Group != "" {
		fmt.Fprintf(&b, "Group=%s\n", s.Group)
	}
	b.WriteString("\n[Install]\nWantedBy=multi-user.target\n")

	if err := os.WriteFile(unitPath, []byte(b.String()), 0644); err != nil {
		return fmt.Errorf("write unit %s: %w (are you root?)", unitPath, err)
	}
	if err := run("systemctl", "daemon-reload"); err != nil {
		return err
	}
	if err := run("systemctl", "enable", "--now", s.Name); err != nil {
		return err
	}
	writeMsShortcut(s)
	return nil
}

// writeMsShortcut creates /usr/local/bin/ms so the user can open the menu by
// typing `ms` from anywhere. Failures are logged but non-fatal — the binary's
// own `<bin> ms` subcommand still works.
func writeMsShortcut(s Spec) {
	if s.ConfigPath == "" || s.BinPath == "" {
		return
	}
	script := fmt.Sprintf("#!/bin/sh\n# mars-shortcut-for=%s\nexec %q ms -config %q \"$@\"\n",
		s.Name, s.BinPath, s.ConfigPath)
	if err := os.WriteFile(msShortcut, []byte(script), 0755); err != nil {
		fmt.Fprintf(os.Stderr, "提示：无法创建 %s 快捷方式（%v）；仍可用 %q ms 进菜单\n", msShortcut, err, s.BinPath)
	}
}

// removeMsShortcutIfOwned deletes the global `ms` wrapper iff it currently
// points at the named service. Prevents uninstalling agent from also blowing
// away a relay's shortcut on a box where both are installed.
func removeMsShortcutIfOwned(name string) {
	b, err := os.ReadFile(msShortcut)
	if err != nil {
		return
	}
	if strings.Contains(string(b), "mars-shortcut-for="+name) {
		_ = os.Remove(msShortcut)
	}
}

func Uninstall(name string) error {
	_ = run("systemctl", "stop", name)
	_ = run("systemctl", "disable", name)
	unitPath := filepath.Join(unitDir, name+".service")
	if err := os.Remove(unitPath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("remove %s: %w", unitPath, err)
	}
	_ = run("systemctl", "daemon-reload")
	removeMsShortcutIfOwned(name)
	return nil
}

func run(name string, args ...string) error {
	cmd := exec.Command(name, args...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("%s %s: %w\n%s", name, strings.Join(args, " "), err, out)
	}
	return nil
}

func QueryStatus(name string) (Status, error) {
	st := Status{}
	unitPath := filepath.Join(unitDir, name+".service")
	if _, err := os.Stat(unitPath); err == nil {
		st.Installed = true
	}

	active := probe("systemctl", "is-active", name)
	st.Running = active == "active" || active == "activating"

	enabled := probe("systemctl", "is-enabled", name)
	st.Enabled = enabled == "enabled" || enabled == "alias" || enabled == "static"

	// One-line synopsis from systemctl status.
	if out, err := exec.Command("systemctl", "show", name,
		"--property=ActiveState,SubState,MainPID", "--value").Output(); err == nil {
		st.Detail = strings.ReplaceAll(strings.TrimSpace(string(out)), "\n", " ")
	}
	return st, nil
}

func Start(name string) error   { return run("systemctl", "start", name) }
func Stop(name string) error    { return run("systemctl", "stop", name) }
func Restart(name string) error { return run("systemctl", "restart", name) }
func Enable(name string) error  { return run("systemctl", "enable", name) }
func Disable(name string) error { return run("systemctl", "disable", name) }

// probe runs a command that may exit non-zero (systemctl is-active returns 3
// when inactive) and returns its trimmed stdout regardless.
func probe(name string, args ...string) string {
	out, _ := exec.Command(name, args...).Output()
	return strings.TrimSpace(string(out))
}

func nonempty(vs ...string) string {
	for _, v := range vs {
		if v != "" {
			return v
		}
	}
	return ""
}
