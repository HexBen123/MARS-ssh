// Package logsink wires the standard `log` output to both stderr and a file
// next to the config. Lightweight rotation keeps one backup (<name>.log.1)
// once the current file passes sizeCap so long-running services don't grow
// unbounded.
package logsink

import (
	"io"
	"log"
	"os"
	"path/filepath"
)

const sizeCap = 10 * 1024 * 1024 // 10 MiB

// Setup opens (or creates) the log file next to configPath and tees log output
// to stderr + file. Returns the file so the caller can keep it alive; safe to
// ignore the return value if the caller doesn't need to close it (process exit
// will flush).
func Setup(configPath, logName string) (*os.File, error) {
	dir := filepath.Dir(configPath)
	if dir == "" {
		dir = "."
	}
	path := filepath.Join(dir, logName)

	if info, err := os.Stat(path); err == nil && info.Size() > sizeCap {
		_ = os.Rename(path, path+".1")
	}

	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return nil, err
	}
	log.SetOutput(io.MultiWriter(os.Stderr, f))
	return f, nil
}
