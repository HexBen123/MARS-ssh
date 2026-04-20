//go:build !windows

package service

import "context"

// MaybeRunAsService is a no-op on non-Windows platforms: systemd and launchd
// start processes conventionally; no SCM handshake is needed.
func MaybeRunAsService(_ string, _ func(ctx context.Context) error) (bool, error) {
	return false, nil
}

// IsRunningAsService is always false off Windows. systemd-managed processes
// use stdin redirected from /dev/null, which the wizard prompt handles fine.
func IsRunningAsService() bool { return false }
