// Package portpool hands out TCP ports from a contiguous range and reclaims
// them when the caller releases. Ports can also be reserved by number (used
// to restore sticky assignments on startup).
package portpool

import (
	"fmt"
	"sync"
)

type Pool struct {
	mu    sync.Mutex
	min   int
	max   int
	used  map[int]bool
	cur   int // scan cursor for next free port
}

func New(min, max int) *Pool {
	if min < 1 {
		min = 1
	}
	if max > 65535 {
		max = 65535
	}
	return &Pool{min: min, max: max, used: map[int]bool{}, cur: min}
}

// Reserve tries to claim a specific port. Returns false if already taken or
// outside the range.
func (p *Pool) Reserve(port int) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	if port < p.min || port > p.max {
		return false
	}
	if p.used[port] {
		return false
	}
	p.used[port] = true
	return true
}

// Allocate claims the next free port in the range.
func (p *Pool) Allocate() (int, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	size := p.max - p.min + 1
	for i := 0; i < size; i++ {
		port := p.min + ((p.cur - p.min + i) % size)
		if !p.used[port] {
			p.used[port] = true
			p.cur = port + 1
			return port, nil
		}
	}
	return 0, fmt.Errorf("port pool exhausted (%d-%d)", p.min, p.max)
}

// Release returns a port to the pool.
func (p *Pool) Release(port int) {
	p.mu.Lock()
	defer p.mu.Unlock()
	delete(p.used, port)
}
