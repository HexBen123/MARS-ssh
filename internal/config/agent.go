package config

import (
	"fmt"
	"os"
	"strings"

	"gopkg.in/yaml.v3"
)

type AgentConfig struct {
	Relay       string `yaml:"relay"`         // "host:port"
	ServerName  string `yaml:"server_name"`   // TLS SNI (usually same as relay host)
	Fingerprint string `yaml:"fingerprint"`   // "sha256:..."
	Token       string `yaml:"token"`         // shared secret given by relay owner
	AgentID     string `yaml:"agent_id"`      // stable ID so relay can pin a port to us
	LocalAddr   string `yaml:"local_addr"`    // what the agent bridges incoming streams to
}

// LoadAgent validates fully for runtime use.
func LoadAgent(path string) (*AgentConfig, error) {
	c, err := parseAgent(path)
	if err != nil {
		return nil, err
	}
	if err := c.validate(true); err != nil {
		return nil, err
	}
	return c, nil
}

// LoadAgentForBootstrap tolerates a missing fingerprint (TOFU will fill it).
func LoadAgentForBootstrap(path string) (*AgentConfig, error) {
	c, err := parseAgent(path)
	if err != nil {
		return nil, err
	}
	if err := c.validate(false); err != nil {
		return nil, err
	}
	return c, nil
}

func parseAgent(path string) (*AgentConfig, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var c AgentConfig
	if err := yaml.Unmarshal(b, &c); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	if c.LocalAddr == "" {
		c.LocalAddr = "127.0.0.1:22"
	}
	return &c, nil
}

func (c *AgentConfig) validate(requireFingerprint bool) error {
	if c.Relay == "" {
		return fmt.Errorf("relay is required (host:port)")
	}
	if c.Token == "" {
		return fmt.Errorf("token is required")
	}
	if c.AgentID == "" {
		return fmt.Errorf("agent_id is required")
	}
	if requireFingerprint {
		if c.Fingerprint == "" {
			return fmt.Errorf("fingerprint is empty (it will be pinned on first run)")
		}
		if !strings.HasPrefix(c.Fingerprint, "sha256:") {
			return fmt.Errorf("fingerprint must start with sha256:")
		}
	}
	return nil
}

func SaveAgent(path string, c *AgentConfig) error {
	b, err := yaml.Marshal(c)
	if err != nil {
		return err
	}
	return os.WriteFile(path, b, 0600)
}

func AgentExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
