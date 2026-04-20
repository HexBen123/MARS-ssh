package protocol

import (
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
)

const (
	MsgHello = "hello"
	MsgOK    = "ok"
	MsgErr   = "err"

	maxFrameLen = 64 * 1024
)

type Hello struct {
	Type     string `json:"type"`      // "hello"
	Token    string `json:"token"`
	AgentID  string `json:"agent_id"`
	Hostname string `json:"hostname,omitempty"`
	OS       string `json:"os,omitempty"`
}

type Response struct {
	Type         string `json:"type"` // "ok" | "err"
	Reason       string `json:"reason,omitempty"`
	AssignedPort int    `json:"assigned_port,omitempty"`
	PublicHost   string `json:"public_host,omitempty"`
}

func WriteJSON(w io.Writer, v any) error {
	payload, err := json.Marshal(v)
	if err != nil {
		return fmt.Errorf("marshal: %w", err)
	}
	if len(payload) > maxFrameLen {
		return fmt.Errorf("frame too large: %d", len(payload))
	}
	var hdr [4]byte
	binary.BigEndian.PutUint32(hdr[:], uint32(len(payload)))
	if _, err := w.Write(hdr[:]); err != nil {
		return err
	}
	_, err = w.Write(payload)
	return err
}

func ReadJSON(r io.Reader, v any) error {
	var hdr [4]byte
	if _, err := io.ReadFull(r, hdr[:]); err != nil {
		return err
	}
	n := binary.BigEndian.Uint32(hdr[:])
	if n == 0 || n > maxFrameLen {
		return fmt.Errorf("invalid frame length: %d", n)
	}
	buf := make([]byte, n)
	if _, err := io.ReadFull(r, buf); err != nil {
		return err
	}
	return json.Unmarshal(buf, v)
}
