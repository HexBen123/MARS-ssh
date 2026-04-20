// Package pubip attempts to discover the host's public IPv4 using a few
// well-known echo endpoints, returning the first success.
package pubip

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"time"
)

// Sources we try in order. Must each return just the IP as plaintext.
// Mix of endpoints so at least one is reachable from most networks
// (including mainland China, where some are blocked).
var sources = []string{
	"https://ipv4.icanhazip.com",
	"https://api.ipify.org",
	"https://ifconfig.me/ip",
	"https://ipinfo.io/ip",
	"https://myip.ipip.net",            // CN-friendly (returns a sentence; we extract IP)
}

// Discover returns the first public IPv4 found, or an error if none responded.
// Total deadline: ~6 seconds across all sources.
func Discover(ctx context.Context) (string, error) {
	deadlineCtx, cancel := context.WithTimeout(ctx, 6*time.Second)
	defer cancel()

	client := &http.Client{
		Timeout: 3 * time.Second,
		Transport: &http.Transport{
			Proxy: http.ProxyFromEnvironment,
			DialContext: (&net.Dialer{
				Timeout: 2 * time.Second,
			}).DialContext,
		},
	}

	var lastErr error
	for _, url := range sources {
		select {
		case <-deadlineCtx.Done():
			if lastErr != nil {
				return "", lastErr
			}
			return "", deadlineCtx.Err()
		default:
		}

		req, err := http.NewRequestWithContext(deadlineCtx, "GET", url, nil)
		if err != nil {
			lastErr = err
			continue
		}
		resp, err := client.Do(req)
		if err != nil {
			lastErr = err
			continue
		}
		body, err := io.ReadAll(io.LimitReader(resp.Body, 1024))
		resp.Body.Close()
		if err != nil {
			lastErr = err
			continue
		}
		if ip := extractIPv4(string(body)); ip != "" {
			return ip, nil
		}
		lastErr = fmt.Errorf("%s: no IPv4 in response", url)
	}
	if lastErr == nil {
		lastErr = fmt.Errorf("no sources available")
	}
	return "", lastErr
}

func extractIPv4(s string) string {
	s = strings.TrimSpace(s)
	if ip := net.ParseIP(s); ip != nil && ip.To4() != nil {
		return ip.To4().String()
	}
	// Some sources wrap the IP in prose; scan tokens for the first IPv4.
	for _, tok := range strings.FieldsFunc(s, func(r rune) bool {
		return r == ' ' || r == '\n' || r == '\t' || r == ',' || r == ':' || r == '\uFF1A'
	}) {
		if ip := net.ParseIP(tok); ip != nil && ip.To4() != nil {
			return ip.To4().String()
		}
	}
	return ""
}
