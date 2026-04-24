package agent

import (
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"runtime"
	"time"

	"github.com/hashicorp/yamux"

	"mars/internal/config"
	"mars/internal/protocol"
	"mars/internal/tlsutil"
)

type Client struct {
	cfg      *config.AgentConfig
	infoPath string // where to write the "last registered" info file
}

// New builds a client. infoPath is where we persist the banner (SSH command,
// assigned port, etc.) so users / install scripts can read it back even when
// the agent is running as a background service. Empty = skip writing.
func New(cfg *config.AgentConfig, infoPath string) *Client {
	return &Client{cfg: cfg, infoPath: infoPath}
}

func (c *Client) Run(ctx context.Context) error {
	backoff := time.Second
	const maxBackoff = 30 * time.Second

	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		err := c.runOnce(ctx)
		if err == nil || ctx.Err() != nil {
			return ctx.Err()
		}
		log.Printf("会话结束：%v；%s 后重连", err, backoff)
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(backoff):
		}
		backoff *= 2
		if backoff > maxBackoff {
			backoff = maxBackoff
		}
	}
}

func (c *Client) runOnce(ctx context.Context) error {
	tlsCfg, err := tlsutil.MakeClientTLS(c.cfg.ServerName, c.cfg.Fingerprint)
	if err != nil {
		return err
	}

	dialer := &net.Dialer{Timeout: 10 * time.Second}
	rawConn, err := dialer.DialContext(ctx, "tcp", c.cfg.Relay)
	if err != nil {
		return fmt.Errorf("dial %s: %w", c.cfg.Relay, err)
	}
	conn := tls.Client(rawConn, tlsCfg)
	hsCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	if err := conn.HandshakeContext(hsCtx); err != nil {
		_ = rawConn.Close()
		return fmt.Errorf("tls handshake: %w", err)
	}

	ymCfg := yamux.DefaultConfig()
	ymCfg.LogOutput = io.Discard
	sess, err := yamux.Client(conn, ymCfg)
	if err != nil {
		_ = conn.Close()
		return fmt.Errorf("yamux client: %w", err)
	}
	defer sess.Close()

	ctrl, err := sess.OpenStream()
	if err != nil {
		return fmt.Errorf("open control stream: %w", err)
	}
	_ = ctrl.SetDeadline(time.Now().Add(10 * time.Second))
	hostname, _ := os.Hostname()
	if err := protocol.WriteJSON(ctrl, protocol.Hello{
		Type:     protocol.MsgHello,
		Token:    c.cfg.Token,
		AgentID:  c.cfg.AgentID,
		Hostname: hostname,
		OS:       runtime.GOOS,
	}); err != nil {
		return fmt.Errorf("send hello: %w", err)
	}
	var resp protocol.Response
	if err := protocol.ReadJSON(ctrl, &resp); err != nil {
		return fmt.Errorf("read hello response: %w", err)
	}
	_ = ctrl.Close()
	if resp.Type != protocol.MsgOK {
		return fmt.Errorf("handshake rejected: %s", resp.Reason)
	}

	host := resp.PublicHost
	if host == "" {
		// relay didn't tell us — fall back to whatever the user put in relay field
		host, _, _ = net.SplitHostPort(c.cfg.Relay)
	}
	banner := fmt.Sprintf("ssh -p %d user@%s", resp.AssignedPort, host)
	log.Printf("=====================================================")
	log.Printf(" 已注册到中转 %s", c.cfg.Relay)
	log.Printf(" AI 或用户现在可以这样连到本机：")
	log.Printf("     %s", banner)
	log.Printf(" 进来的流量会桥接到 %s", c.cfg.LocalAddr)
	log.Printf("=====================================================")

	// Also write the last-known SSH string to a file next to the config so the
	// user can grep it any time (useful when running as a service).
	if c.infoPath != "" {
		_ = writeInfoFile(c.infoPath, c.cfg, resp, banner)
	}

	go func() {
		<-ctx.Done()
		_ = sess.Close()
	}()

	for {
		stream, err := sess.AcceptStream()
		if err != nil {
			return fmt.Errorf("accept stream: %w", err)
		}
		go c.handleStream(stream)
	}
}

func (c *Client) handleStream(stream io.ReadWriteCloser) {
	local, err := net.DialTimeout("tcp", c.cfg.LocalAddr, 5*time.Second)
	if err != nil {
		log.Printf("dial %s: %v", c.cfg.LocalAddr, err)
		_ = stream.Close()
		return
	}
	copyBoth(local, stream)
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

func writeInfoFile(path string, cfg *config.AgentConfig, resp protocol.Response, banner string) error {
	content := fmt.Sprintf(
		"最后成功注册时间：%s\n"+
			"中转地址：   %s\n"+
			"公开域名：   %s\n"+
			"分配端口：   %d\n"+
			"SSH 命令：   %s\n",
		time.Now().Local().Format("2006-01-02 15:04:05"),
		cfg.Relay, resp.PublicHost, resp.AssignedPort, banner,
	)
	return os.WriteFile(path, []byte(content), 0644)
}
