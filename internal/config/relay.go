package config

import (
	"fmt"
	"os"

	"gopkg.in/yaml.v3"
)

type RelayConfig struct {
	Listen     string    `yaml:"listen"`       // ":7000"
	PublicHost string    `yaml:"public_host"`  // domain or public IP shown to users
	Token      string    `yaml:"token"`        // shared secret for all agents
	TLS        TLSFiles  `yaml:"tls"`
	PortRange  PortRange `yaml:"port_range"`   // pool of public SSH ports to allocate from
	StateFile  string    `yaml:"state_file"`   // JSON state for sticky agent→port mapping
}

type TLSFiles struct {
	Cert string `yaml:"cert"`
	Key  string `yaml:"key"`
}

type PortRange struct {
	Min int `yaml:"min"`
	Max int `yaml:"max"`
}

func LoadRelay(path string) (*RelayConfig, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var c RelayConfig
	if err := yaml.Unmarshal(b, &c); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	if c.StateFile == "" {
		c.StateFile = "state.json"
	}
	if err := c.validate(); err != nil {
		return nil, err
	}
	return &c, nil
}

func (c *RelayConfig) validate() error {
	if c.Listen == "" {
		return fmt.Errorf("listen is required")
	}
	if c.Token == "" {
		return fmt.Errorf("token is required")
	}
	if c.TLS.Cert == "" || c.TLS.Key == "" {
		return fmt.Errorf("tls.cert and tls.key are required")
	}
	if c.PortRange.Min <= 0 || c.PortRange.Max <= 0 || c.PortRange.Min > c.PortRange.Max {
		return fmt.Errorf("port_range.min/max invalid")
	}
	if c.PortRange.Min < 1024 {
		return fmt.Errorf("port_range.min must be >= 1024")
	}
	if c.PortRange.Max > 65535 {
		return fmt.Errorf("port_range.max must be <= 65535")
	}
	return nil
}

func SaveRelay(path string, c *RelayConfig) error {
	b, err := yaml.Marshal(c)
	if err != nil {
		return err
	}
	return os.WriteFile(path, b, 0600)
}

func RelayExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
