package tlsutil

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"fmt"
	"net"
	"os"
	"time"
)

// FetchFingerprint connects to addr with TLS, bypassing verification, and
// returns "sha256:<hex>" of the peer's leaf certificate DER. Used by agent
// bootstrap (TOFU) and by relay `issue` (derives it from the local cert file).
func FetchFingerprint(ctx context.Context, addr, serverName string) (string, error) {
	d := &net.Dialer{Timeout: 10 * time.Second}
	raw, err := d.DialContext(ctx, "tcp", addr)
	if err != nil {
		return "", fmt.Errorf("dial %s: %w", addr, err)
	}
	defer raw.Close()

	conn := tls.Client(raw, &tls.Config{
		ServerName:         serverName,
		InsecureSkipVerify: true,
		MinVersion:         tls.VersionTLS12,
	})
	hsCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	if err := conn.HandshakeContext(hsCtx); err != nil {
		return "", fmt.Errorf("tls handshake: %w", err)
	}
	state := conn.ConnectionState()
	if len(state.PeerCertificates) == 0 {
		return "", fmt.Errorf("server presented no certificate")
	}
	sum := sha256.Sum256(state.PeerCertificates[0].Raw)
	return "sha256:" + hex.EncodeToString(sum[:]), nil
}

// FingerprintFromFile computes "sha256:<hex>" of the first CERTIFICATE block
// in certFile. Used by `relay issue` to embed the fingerprint into generated
// agent configs without needing a network round trip.
func FingerprintFromFile(certFile string) (string, error) {
	b, err := os.ReadFile(certFile)
	if err != nil {
		return "", err
	}
	block, _ := pemDecode(b)
	if block == nil {
		return "", fmt.Errorf("no CERTIFICATE block in %s", certFile)
	}
	sum := sha256.Sum256(block.Bytes)
	return "sha256:" + hex.EncodeToString(sum[:]), nil
}
