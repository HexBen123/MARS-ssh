//go:build windows

package service

import (
	"context"

	"golang.org/x/sys/windows/svc"
)

// IsRunningAsService reports whether this process was started by the Windows
// Service Control Manager. Used by callers to skip interactive prompts.
func IsRunningAsService() bool {
	ok, err := svc.IsWindowsService()
	return err == nil && ok
}

// MaybeRunAsService checks if the current process was started by the Windows
// Service Control Manager. If so, it runs under svc.Run, calling `run` inside
// a goroutine and handling Stop/Shutdown by cancelling the context. Returns
// (true, err) if it ran under SCM. Returns (false, nil) if not a service
// context — caller should then run in the foreground.
func MaybeRunAsService(name string, run func(ctx context.Context) error) (bool, error) {
	isSvc, err := svc.IsWindowsService()
	if err != nil || !isSvc {
		return false, nil
	}
	h := &handler{run: run}
	if err := svc.Run(name, h); err != nil {
		return true, err
	}
	return true, h.runErr
}

type handler struct {
	run    func(ctx context.Context) error
	runErr error
}

func (h *handler) Execute(_ []string, r <-chan svc.ChangeRequest, s chan<- svc.Status) (bool, uint32) {
	const accepts = svc.AcceptStop | svc.AcceptShutdown
	s <- svc.Status{State: svc.StartPending}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{})
	go func() {
		h.runErr = h.run(ctx)
		close(done)
	}()

	s <- svc.Status{State: svc.Running, Accepts: accepts}

	for {
		select {
		case c := <-r:
			switch c.Cmd {
			case svc.Interrogate:
				s <- c.CurrentStatus
			case svc.Stop, svc.Shutdown:
				s <- svc.Status{State: svc.StopPending}
				cancel()
				<-done
				s <- svc.Status{State: svc.Stopped}
				return false, 0
			}
		case <-done:
			s <- svc.Status{State: svc.Stopped}
			return false, 0
		}
	}
}
