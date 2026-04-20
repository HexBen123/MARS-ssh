package tlsutil

import "encoding/pem"

// pemDecode finds the first CERTIFICATE block in data, skipping anything else
// (e.g., a private key block that may be bundled in the same file).
func pemDecode(data []byte) (*pem.Block, []byte) {
	rest := data
	for {
		block, next := pem.Decode(rest)
		if block == nil {
			return nil, rest
		}
		if block.Type == "CERTIFICATE" {
			return block, next
		}
		rest = next
	}
}
