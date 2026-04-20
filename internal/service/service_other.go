//go:build !linux && !windows

package service

import "fmt"

func Install(Spec) error     { return fmt.Errorf("service install not supported on this OS") }
func Uninstall(string) error { return fmt.Errorf("service uninstall not supported on this OS") }

func QueryStatus(string) (Status, error) { return Status{}, nil }
func Start(string) error                  { return fmt.Errorf("not supported on this OS") }
func Stop(string) error                   { return fmt.Errorf("not supported on this OS") }
func Restart(string) error                { return fmt.Errorf("not supported on this OS") }
func Enable(string) error                 { return fmt.Errorf("not supported on this OS") }
func Disable(string) error                { return fmt.Errorf("not supported on this OS") }
