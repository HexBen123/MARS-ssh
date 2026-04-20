//go:build windows

package service

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"golang.org/x/sys/windows"
	"golang.org/x/sys/windows/svc"
	"golang.org/x/sys/windows/svc/mgr"
)

func Install(s Spec) error {
	m, err := mgr.Connect()
	if err != nil {
		return fmt.Errorf("connect SCM: %w (run as Administrator)", err)
	}
	defer m.Disconnect()

	if existing, err := m.OpenService(s.Name); err == nil {
		_ = existing.Close()
		return fmt.Errorf("service %q already exists; run `uninstall` first", s.Name)
	}

	cfg := mgr.Config{
		ServiceType:      windows.SERVICE_WIN32_OWN_PROCESS,
		StartType:        mgr.StartAutomatic,
		ErrorControl:     mgr.ErrorNormal,
		DisplayName:      nonempty(s.DisplayName, s.Name),
		Description:      s.Description,
		DelayedAutoStart: false,
	}
	svcObj, err := m.CreateService(s.Name, s.BinPath, cfg, s.Args...)
	if err != nil {
		return fmt.Errorf("create service: %w", err)
	}
	defer svcObj.Close()

	// Restart on failure: wait 3s twice, then 5s.
	recovery := []mgr.RecoveryAction{
		{Type: mgr.ServiceRestart, Delay: 3 * time.Second},
		{Type: mgr.ServiceRestart, Delay: 3 * time.Second},
		{Type: mgr.ServiceRestart, Delay: 5 * time.Second},
	}
	_ = svcObj.SetRecoveryActions(recovery, 86400)

	if err := svcObj.Start(); err != nil {
		return fmt.Errorf("start service: %w (check Windows event log)", err)
	}
	writeMsShortcut(s)
	return nil
}

// writeMsShortcut drops `ms.cmd` beside the binary so users can open the menu
// with `ms` once that directory is on PATH. Silent failure — `<bin>.exe ms`
// still works regardless.
func writeMsShortcut(s Spec) {
	if s.ConfigPath == "" || s.BinPath == "" {
		return
	}
	dir := filepath.Dir(s.BinPath)
	path := filepath.Join(dir, "ms.cmd")
	script := fmt.Sprintf("@echo off\r\nREM mars-shortcut-for=%s\r\n\"%s\" ms -config \"%s\" %%*\r\n",
		s.Name, s.BinPath, s.ConfigPath)
	if err := os.WriteFile(path, []byte(script), 0644); err == nil {
		fmt.Printf("提示：已生成 %s；把目录 %s 加到 PATH 后 `ms` 就能全局调用\n", path, dir)
	}
}

func removeMsShortcutIfOwned(binPath, name string) {
	if binPath == "" {
		return
	}
	path := filepath.Join(filepath.Dir(binPath), "ms.cmd")
	b, err := os.ReadFile(path)
	if err != nil {
		return
	}
	if strings.Contains(string(b), "mars-shortcut-for="+name) {
		_ = os.Remove(path)
	}
}

func Uninstall(name string) error {
	m, err := mgr.Connect()
	if err != nil {
		return fmt.Errorf("connect SCM: %w (run as Administrator)", err)
	}
	defer m.Disconnect()

	svcObj, err := m.OpenService(name)
	if err != nil {
		if strings.Contains(err.Error(), "does not exist") {
			return nil
		}
		return fmt.Errorf("open service %q: %w", name, err)
	}
	defer svcObj.Close()

	// Grab the binary path before we delete, so we can clean up ms.cmd next to it.
	var binPath string
	if cfg, err := svcObj.Config(); err == nil {
		binPath = extractExePath(cfg.BinaryPathName)
	}

	// Best-effort stop, then delete.
	status, _ := svcObj.Control(svc.Stop)
	_ = status
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		s, err := svcObj.Query()
		if err != nil || s.State == svc.Stopped {
			break
		}
		time.Sleep(200 * time.Millisecond)
	}
	if err := svcObj.Delete(); err != nil {
		return fmt.Errorf("delete service: %w", err)
	}
	removeMsShortcutIfOwned(binPath, name)
	return nil
}

// extractExePath pulls the .exe path out of an SCM BinaryPathName, which is a
// command-line like `"C:\Program Files\MARS\agent.exe" run -config ...` or
// `C:\bin\agent.exe run ...`. We only need the first token.
func extractExePath(cmdline string) string {
	s := strings.TrimSpace(cmdline)
	if s == "" {
		return ""
	}
	if s[0] == '"' {
		if i := strings.Index(s[1:], `"`); i >= 0 {
			return s[1 : 1+i]
		}
		return ""
	}
	if i := strings.IndexByte(s, ' '); i >= 0 {
		return s[:i]
	}
	return s
}

func QueryStatus(name string) (Status, error) {
	st := Status{}
	m, err := mgr.Connect()
	if err != nil {
		return st, fmt.Errorf("connect SCM: %w", err)
	}
	defer m.Disconnect()

	svcObj, err := m.OpenService(name)
	if err != nil {
		if strings.Contains(err.Error(), "does not exist") {
			return st, nil
		}
		return st, fmt.Errorf("open service %q: %w", name, err)
	}
	defer svcObj.Close()
	st.Installed = true

	if s, err := svcObj.Query(); err == nil {
		switch s.State {
		case svc.Running, svc.StartPending:
			st.Running = true
			st.Detail = "running"
		case svc.StopPending:
			st.Detail = "stopping"
		case svc.PausePending, svc.Paused, svc.ContinuePending:
			st.Detail = "paused"
		default:
			st.Detail = "stopped"
		}
	}
	if cfg, err := svcObj.Config(); err == nil {
		st.Enabled = cfg.StartType == mgr.StartAutomatic
	}
	return st, nil
}

func Start(name string) error {
	return withService(name, func(s *mgr.Service) error { return s.Start() })
}

func Stop(name string) error {
	return withService(name, func(s *mgr.Service) error {
		if _, err := s.Control(svc.Stop); err != nil {
			return err
		}
		deadline := time.Now().Add(10 * time.Second)
		for time.Now().Before(deadline) {
			st, err := s.Query()
			if err != nil || st.State == svc.Stopped {
				return nil
			}
			time.Sleep(200 * time.Millisecond)
		}
		return fmt.Errorf("timeout waiting for %q to stop", name)
	})
}

func Restart(name string) error {
	if err := Stop(name); err != nil {
		return err
	}
	return Start(name)
}

func Enable(name string) error  { return setStartType(name, mgr.StartAutomatic) }
func Disable(name string) error { return setStartType(name, mgr.StartManual) }

func setStartType(name string, start uint32) error {
	return withService(name, func(s *mgr.Service) error {
		cfg, err := s.Config()
		if err != nil {
			return err
		}
		cfg.StartType = start
		return s.UpdateConfig(cfg)
	})
}

func withService(name string, fn func(*mgr.Service) error) error {
	m, err := mgr.Connect()
	if err != nil {
		return fmt.Errorf("connect SCM: %w (run as Administrator)", err)
	}
	defer m.Disconnect()
	svcObj, err := m.OpenService(name)
	if err != nil {
		return fmt.Errorf("open service %q: %w", name, err)
	}
	defer svcObj.Close()
	return fn(svcObj)
}

func nonempty(vs ...string) string {
	for _, v := range vs {
		if v != "" {
			return v
		}
	}
	return ""
}
