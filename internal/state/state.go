// Package state persists the agent→port assignment map so a relay restart
// (or an agent reconnect) keeps the same public port for the same agent.
package state

import (
	"encoding/json"
	"os"
	"sync"
	"time"
)

type Entry struct {
	Port     int       `json:"port"`
	Hostname string    `json:"hostname,omitempty"`
	LastSeen time.Time `json:"last_seen"`
}

type Store struct {
	mu      sync.Mutex
	path    string
	Agents  map[string]Entry `json:"agents"`
}

func Load(path string) (*Store, error) {
	s := &Store{path: path, Agents: map[string]Entry{}}
	b, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return s, nil
	}
	if err != nil {
		return nil, err
	}
	if len(b) == 0 {
		return s, nil
	}
	if err := json.Unmarshal(b, s); err != nil {
		return nil, err
	}
	if s.Agents == nil {
		s.Agents = map[string]Entry{}
	}
	return s, nil
}

// Get returns the stored entry, if any.
func (s *Store) Get(agentID string) (Entry, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	e, ok := s.Agents[agentID]
	return e, ok
}

// Put updates the entry and persists immediately. Errors are returned so the
// caller can decide whether persistence loss is fatal (usually not).
func (s *Store) Put(agentID string, e Entry) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	e.LastSeen = time.Now().UTC()
	s.Agents[agentID] = e
	return s.saveLocked()
}

func (s *Store) saveLocked() error {
	b, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	tmp := s.path + ".tmp"
	if err := os.WriteFile(tmp, b, 0600); err != nil {
		return err
	}
	return os.Rename(tmp, s.path)
}
