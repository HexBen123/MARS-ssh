package relay

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"sync"
	"time"

	"github.com/hashicorp/yamux"

	"mars/internal/config"
	"mars/internal/portpool"
	"mars/internal/protocol"
	"mars/internal/state"
	"mars/internal/tlsutil"
)

type Server struct {
	cfg   *config.RelayConfig
	pool  *portpool.Pool
	store *state.Store

	mu       sync.Mutex
	sessions map[string]*agentSession // by agent_id
}

type agentSession struct {
	agentID string
	session *yamux.Session
	ln      net.Listener
	port    int
}

func New(cfg *config.RelayConfig, store *state.Store) *Server {
	pool := portpool.New(cfg.PortRange.Min, cfg.PortRange.Max)
	// Re-reserve any ports that were previously assigned, so new agents don't
	// land on someone else's sticky port.
	for _, e := range store.Agents {
		pool.Reserve(e.Port)
	}
	return &Server{cfg: cfg, pool: pool, store: store, sessions: map[string]*agentSession{}}
}

func (s *Server) Run(ctx context.Context) error {
	tlsCfg, err := tlsutil.LoadServerCert(s.cfg.TLS.Cert, s.cfg.TLS.Key)
	if err != nil {
		return err
	}
	ln, err := tls.Listen("tcp", s.cfg.Listen, tlsCfg)
	if err != nil {
		return fmt.Errorf("listen %s: %w", s.cfg.Listen, err)
	}
	log.Printf("中转已监听 %s（TLS）；公开地址 %q；端口池 %d-%d",
		s.cfg.Listen, s.cfg.PublicHost, s.cfg.PortRange.Min, s.cfg.PortRange.Max)

	go func() {
		<-ctx.Done()
		_ = ln.Close()
	}()

	for {
		conn, err := ln.Accept()
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			log.Printf("accept: %v", err)
			continue
		}
		go s.handleConn(ctx, conn)
	}
}

func (s *Server) handleConn(ctx context.Context, conn net.Conn) {
	remote := conn.RemoteAddr().String()

	_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
	if tc, ok := conn.(*tls.Conn); ok {
		if err := tc.HandshakeContext(ctx); err != nil {
			log.Printf("[%s] tls handshake: %v", remote, err)
			_ = conn.Close()
			return
		}
	}
	_ = conn.SetDeadline(time.Time{})

	ymCfg := yamux.DefaultConfig()
	ymCfg.LogOutput = io.Discard
	sess, err := yamux.Server(conn, ymCfg)
	if err != nil {
		log.Printf("[%s] yamux server: %v", remote, err)
		_ = conn.Close()
		return
	}

	ctrl, err := sess.AcceptStream()
	if err != nil {
		log.Printf("[%s] accept control stream: %v", remote, err)
		_ = sess.Close()
		return
	}
	_ = ctrl.SetDeadline(time.Now().Add(10 * time.Second))

	var hello protocol.Hello
	if err := protocol.ReadJSON(ctrl, &hello); err != nil {
		log.Printf("[%s] read hello: %v", remote, err)
		_ = sess.Close()
		return
	}

	if hello.Type != protocol.MsgHello || hello.Token != s.cfg.Token || hello.AgentID == "" {
		log.Printf("[%s] 鉴权失败（agent_id=%q）", remote, hello.AgentID)
		_ = protocol.WriteJSON(ctrl, protocol.Response{Type: protocol.MsgErr, Reason: "unauthorized"})
		_ = ctrl.Close()
		_ = sess.Close()
		return
	}

	port, err := s.claimPort(hello.AgentID)
	if err != nil {
		log.Printf("[%s] port claim for %q failed: %v", remote, hello.AgentID, err)
		_ = protocol.WriteJSON(ctrl, protocol.Response{Type: protocol.MsgErr, Reason: err.Error()})
		_ = ctrl.Close()
		_ = sess.Close()
		return
	}

	// Persist the sticky mapping.
	_ = s.store.Put(hello.AgentID, state.Entry{Port: port, Hostname: hello.Hostname})

	if err := protocol.WriteJSON(ctrl, protocol.Response{
		Type:         protocol.MsgOK,
		AssignedPort: port,
		PublicHost:   s.cfg.PublicHost,
	}); err != nil {
		s.pool.Release(port)
		log.Printf("[%s] write ok: %v", remote, err)
		_ = sess.Close()
		return
	}
	_ = ctrl.Close()

	if err := s.serveAgent(ctx, hello.AgentID, port, sess, remote); err != nil {
		log.Printf("[%s][%s] session ended: %v", remote, hello.AgentID, err)
	}
}

// claimPort returns the previously-assigned port if known, otherwise pulls a
// fresh one from the pool. Returns an error if the pool is exhausted.
//
// The stored port is trusted: state.json is the authoritative mapping for
// agent_id→port, and we pre-reserve all stored ports at startup. On reconnect
// the port is still reserved for *this* agent, so we simply return it without
// re-reserving.
func (s *Server) claimPort(agentID string) (int, error) {
	if prev, ok := s.store.Get(agentID); ok && prev.Port > 0 {
		s.pool.Reserve(prev.Port) // no-op if already reserved; covers first boot after state load
		return prev.Port, nil
	}
	return s.pool.Allocate()
}

func (s *Server) serveAgent(ctx context.Context, agentID string, port int, sess *yamux.Session, remote string) error {
	s.mu.Lock()
	if prev, ok := s.sessions[agentID]; ok {
		log.Printf("[%s] 替换 %q 的旧会话", remote, agentID)
		_ = prev.session.Close()
		if prev.ln != nil {
			_ = prev.ln.Close()
		}
		// Do NOT release prev.port — we're reusing it for the new session.
		delete(s.sessions, agentID)
	}

	addr := fmt.Sprintf(":%d", port)
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		s.mu.Unlock()
		s.pool.Release(port)
		_ = sess.Close()
		return fmt.Errorf("listen %s: %w", addr, err)
	}
	as := &agentSession{agentID: agentID, session: sess, ln: ln, port: port}
	s.sessions[agentID] = as
	s.mu.Unlock()

	log.Printf("目标 %q 从 %s 接入，开放 %s", agentID, remote, addr)

	defer func() {
		s.mu.Lock()
		if cur, ok := s.sessions[agentID]; ok && cur == as {
			delete(s.sessions, agentID)
		}
		s.mu.Unlock()
		_ = ln.Close()
		_ = sess.Close()
		// Keep the port reserved in the pool so the same agent gets it back
		// on reconnect. It's released only when the state entry is removed
		// (which we don't do automatically in v1).
		log.Printf("目标 %q 已断开，关闭 %s（端口保留以便重连）", agentID, addr)
	}()

	sessDone := make(chan struct{})
	go func() {
		<-sess.CloseChan()
		_ = ln.Close()
		close(sessDone)
	}()
	go func() {
		<-ctx.Done()
		_ = ln.Close()
	}()

	for {
		clientConn, err := ln.Accept()
		if err != nil {
			if errors.Is(err, net.ErrClosed) {
				<-sessDone
				return nil
			}
			return err
		}
		go s.bridge(sess, clientConn, agentID)
	}
}

func (s *Server) bridge(sess *yamux.Session, clientConn net.Conn, agentID string) {
	stream, err := sess.OpenStream()
	if err != nil {
		log.Printf("[%s] open stream: %v", agentID, err)
		_ = clientConn.Close()
		return
	}
	copyBoth(clientConn, stream)
}

func copyBoth(a, b io.ReadWriteCloser) {
	done := make(chan struct{}, 2)
	go func() { _, _ = io.Copy(a, b); done <- struct{}{} }()
	go func() { _, _ = io.Copy(b, a); done <- struct{}{} }()
	<-done
	_ = a.Close()
	_ = b.Close()
	<-done
}
