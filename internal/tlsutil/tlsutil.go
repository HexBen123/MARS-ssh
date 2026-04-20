package tlsutil

import (
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"fmt"
	"strings"
)

func LoadServerCert(certFile, keyFile string) (*tls.Config, error) {
	cert, err := tls.LoadX509KeyPair(certFile, keyFile)
	if err != nil {
		return nil, fmt.Errorf("load keypair: %w", err)
	}
	return &tls.Config{
		Certificates: []tls.Certificate{cert},
		MinVersion:   tls.VersionTLS12,
	}, nil
}

// MakeClientTLS returns a TLS config that skips hostname verification and
// instead pins the server's certificate by the SHA-256 of its DER bytes.
// Fingerprint format: "sha256:<hex>" (case-insensitive, colons stripped).
func MakeClientTLS(serverName, fingerprint string) (*tls.Config, error) {
	want, err := parseFingerprint(fingerprint)
	if err != nil {
		return nil, err
	}
	return &tls.Config{
		ServerName:         serverName,
		InsecureSkipVerify: true,
		MinVersion:         tls.VersionTLS12,
		VerifyPeerCertificate: func(raw [][]byte, _ [][]*x509.Certificate) error {
			if len(raw) == 0 {
				return fmt.Errorf("no peer certificate")
			}
			got := sha256.Sum256(raw[0])
			if !equalBytes(got[:], want) {
				return fmt.Errorf("certificate fingerprint mismatch: got sha256:%s", hex.EncodeToString(got[:]))
			}
			return nil
		},
	}, nil
}

func parseFingerprint(s string) ([]byte, error) {
	s = strings.TrimPrefix(strings.ToLower(s), "sha256:")
	s = strings.ReplaceAll(s, ":", "")
	b, err := hex.DecodeString(s)
	if err != nil {
		return nil, fmt.Errorf("decode fingerprint: %w", err)
	}
	if len(b) != sha256.Size {
		return nil, fmt.Errorf("fingerprint must be %d bytes, got %d", sha256.Size, len(b))
	}
	return b, nil
}

func equalBytes(a, b []byte) bool {
	if len(a) != len(b) {
		return false
	}
	var diff byte
	for i := range a {
		diff |= a[i] ^ b[i]
	}
	return diff == 0
}
