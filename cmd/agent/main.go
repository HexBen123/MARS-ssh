package main

import (
	"bufio"
	"context"
	"crypto/rand"
	"encoding/hex"
	"flag"
	"fmt"
	"log"
	"net"
	"os"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"

	"mars/internal/agent"
	"mars/internal/config"
	"mars/internal/logsink"
	"mars/internal/menu"
	"mars/internal/service"
	"mars/internal/tlsutil"
)

const logFileName = "mars-agent.log"

const (
	serviceName        = "mars-agent"
	serviceDisplayName = "MARS Reverse SSH Tunnel Agent"
	serviceDescription = "MARS (Minimal AI Reverse Ssh) 目标代理"
)

const usage = `MARS agent —— 反向 SSH 隧道目标端

用法：
  agent [-config <路径>]              启动（首次运行进入交互向导）
  agent run [-config <路径>]          同上
  agent ms [-config <路径>]           打开服务管理菜单
  agent install [-config <路径>]      注册为系统服务并启动
  agent uninstall                     停止服务并移除注册
`

func main() {
	log.SetFlags(log.LstdFlags | log.Lmicroseconds)

	args := os.Args[1:]
	cmd := "run"
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		cmd = args[0]
		args = args[1:]
	}

	switch cmd {
	case "run":
		cmdRun(args)
	case "ms", "menu":
		cmdMenu(args)
	case "install":
		cmdInstall(args)
	case "uninstall":
		cmdUninstall(args)
	case "help", "-h", "--help":
		fmt.Print(usage)
	default:
		fmt.Fprintf(os.Stderr, "未知命令 %q\n\n%s", cmd, usage)
		os.Exit(2)
	}
}

func cmdRun(args []string) {
	fs := flag.NewFlagSet("run", flag.ExitOnError)
	cfgPath := fs.String("config", "agent.yaml", "path to agent config yaml")
	_ = fs.Parse(args)

	if !config.AgentExists(*cfgPath) && !service.IsRunningAsService() {
		if err := agentWizard(*cfgPath); err != nil {
			log.Fatalf("初始化失败：%v", err)
		}
	}

	if _, err := logsink.Setup(*cfgPath, logFileName); err != nil {
		log.Printf("提示：无法打开日志文件（%v），仅输出到 stderr", err)
	}

	run := func(ctx context.Context) error {
		cfg, err := config.LoadAgent(*cfgPath)
		if err != nil {
			return err
		}
		return agent.New(cfg).Run(ctx)
	}

	if ok, err := service.MaybeRunAsService(serviceName, run); ok {
		if err != nil {
			log.Fatalf("服务运行失败：%v", err)
		}
		return
	}

	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := run(ctx); err != nil && ctx.Err() == nil {
		log.Fatalf("agent 运行失败：%v", err)
	}
}

func agentWizard(cfgPath string) error {
	return runAgentWizard(cfgPath, nil)
}

// runAgentWizard drives the prompt flow. If `existing` is non-nil its fields
// become the defaults (used by the "修改配置" menu action).
func runAgentWizard(cfgPath string, existing *config.AgentConfig) error {
	in := bufio.NewReader(os.Stdin)
	fmt.Println("=====================================================")
	if existing == nil {
		fmt.Println(" MARS 目标代理 —— 首次启动向导")
	} else {
		fmt.Println(" MARS 目标代理 —— 修改配置")
	}
	fmt.Println(" （直接回车使用方括号里的默认值）")
	fmt.Println("=====================================================")

	defaultRelay, defaultToken, defaultLocal := "", "", "127.0.0.1:22"
	if existing != nil {
		defaultRelay = existing.Relay
		defaultToken = existing.Token
		if existing.LocalAddr != "" {
			defaultLocal = existing.LocalAddr
		}
	}

	relayAddr := promptString(in, "中转地址（host:port）", defaultRelay)
	if relayAddr == "" {
		return fmt.Errorf("必须填写中转地址")
	}
	host, _, err := net.SplitHostPort(relayAddr)
	if err != nil {
		return fmt.Errorf("中转地址必须是 host:port 格式：%w", err)
	}

	token := promptString(in, "令牌（从中转方复制过来）", defaultToken)
	if token == "" {
		return fmt.Errorf("必须填写令牌")
	}

	localAddr := promptString(in, "要暴露的本地服务地址", defaultLocal)

	// Keep agent_id stable on edits so the relay-side sticky port still applies.
	agentID := ""
	if existing != nil {
		agentID = existing.AgentID
	}
	if agentID == "" {
		hn, _ := os.Hostname()
		if hn == "" {
			hn = "agent"
		}
		suffix := make([]byte, 3)
		if _, err := rand.Read(suffix); err != nil {
			return fmt.Errorf("生成 agent_id 失败：%w", err)
		}
		agentID = sanitizeHostname(hn) + "-" + hex.EncodeToString(suffix)
	}

	dir := filepath.Dir(cfgPath)
	if dir == "" {
		dir = "."
	}
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("创建目录 %s 失败：%w", dir, err)
	}

	// Re-fetch the TLS fingerprint whenever the relay address changes or we
	// have no existing pin yet.
	fp := ""
	if existing != nil {
		fp = existing.Fingerprint
	}
	if fp == "" || (existing != nil && existing.Relay != relayAddr) {
		fmt.Printf("正在从 %s 获取 TLS 指纹 ... ", relayAddr)
		fpCtx, cancel := context.WithCancel(context.Background())
		defer cancel()
		newFp, err := tlsutil.FetchFingerprint(fpCtx, relayAddr, host)
		if err != nil {
			fmt.Println("失败")
			return fmt.Errorf("获取指纹失败：%w", err)
		}
		fmt.Println("完成")
		fmt.Printf("  已钉扎：%s\n", newFp)
		fp = newFp
	}

	cfg := &config.AgentConfig{
		Relay:       relayAddr,
		ServerName:  host,
		Fingerprint: fp,
		Token:       token,
		AgentID:     agentID,
		LocalAddr:   localAddr,
	}
	if err := config.SaveAgent(cfgPath, cfg); err != nil {
		return fmt.Errorf("保存配置失败：%w", err)
	}

	fmt.Println()
	fmt.Println("=====================================================")
	fmt.Printf(" 配置已保存到 %s （agent_id=%s）\n", cfgPath, agentID)
	if existing == nil {
		fmt.Println(" 小贴士：以后想改配置 / 启停服务，跑 `<本程序路径> ms`")
		fmt.Println("        `sudo <本程序> install` 之后，`ms` 会成为全局命令。")
		fmt.Println(" 正在连接中转 ...")
	}
	fmt.Println("=====================================================")
	fmt.Println()
	return nil
}

func sanitizeHostname(s string) string {
	var b strings.Builder
	for _, r := range s {
		switch {
		case r >= 'a' && r <= 'z', r >= '0' && r <= '9', r == '-':
			b.WriteRune(r)
		case r >= 'A' && r <= 'Z':
			b.WriteRune(r + ('a' - 'A'))
		default:
			b.WriteRune('-')
		}
	}
	out := b.String()
	if out == "" {
		return "agent"
	}
	return out
}

func cmdMenu(args []string) {
	fs := flag.NewFlagSet("ms", flag.ExitOnError)
	cfgPath := fs.String("config", "agent.yaml", "path to agent config yaml")
	_ = fs.Parse(args)

	absCfg, err := filepath.Abs(*cfgPath)
	if err != nil {
		log.Fatalf("解析配置路径失败：%v", err)
	}
	if !config.AgentExists(absCfg) {
		fmt.Println("还没有配置文件，先跑一次初始化向导。")
		if err := runAgentWizard(absCfg, nil); err != nil {
			log.Fatalf("初始化失败：%v", err)
		}
		fmt.Println("向导完成，现在进入管理菜单。")
	}

	bin, err := os.Executable()
	if err != nil {
		log.Fatalf("定位自身可执行文件失败：%v", err)
	}
	sp := menu.Spec{
		Title:       "MARS 目标代理（agent）",
		ServiceName: serviceName,
		ConfigPath:  absCfg,
		Binary:      bin,
		InstallSpec: service.Spec{
			Name:        serviceName,
			DisplayName: serviceDisplayName,
			Description: serviceDescription,
			BinPath:     bin,
			ConfigPath:  absCfg,
			Args:        []string{"run", "-config", absCfg},
		},
		EditConfig: func() error {
			cur, err := config.LoadAgentForBootstrap(absCfg)
			if err != nil {
				return err
			}
			return runAgentWizard(absCfg, cur)
		},
		RunForeground: func() error {
			if _, err := logsink.Setup(absCfg, logFileName); err != nil {
				log.Printf("提示：无法打开日志文件（%v），仅输出到 stderr", err)
			}
			ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
			defer cancel()
			cfg, err := config.LoadAgent(absCfg)
			if err != nil {
				return err
			}
			return agent.New(cfg).Run(ctx)
		},
	}
	if err := menu.Run(sp); err != nil {
		log.Fatalf("菜单退出：%v", err)
	}
}

func cmdInstall(args []string) {
	fs := flag.NewFlagSet("install", flag.ExitOnError)
	cfgPath := fs.String("config", "agent.yaml", "path to agent config yaml")
	_ = fs.Parse(args)

	absCfg, err := filepath.Abs(*cfgPath)
	if err != nil {
		log.Fatalf("解析配置路径失败：%v", err)
	}
	if !config.AgentExists(absCfg) {
		if err := agentWizard(absCfg); err != nil {
			log.Fatalf("初始化失败：%v", err)
		}
	}
	if _, err := config.LoadAgent(absCfg); err != nil {
		log.Fatalf("配置校验失败：%v", err)
	}
	bin, err := os.Executable()
	if err != nil {
		log.Fatalf("定位自身可执行文件失败：%v", err)
	}
	spec := service.Spec{
		Name:        serviceName,
		DisplayName: serviceDisplayName,
		Description: serviceDescription,
		BinPath:     bin,
		ConfigPath:  absCfg,
		Args:        []string{"run", "-config", absCfg},
	}
	if err := service.Install(spec); err != nil {
		log.Fatalf("注册服务失败：%v", err)
	}
	fmt.Printf("服务 %q 已注册并启动（配置：%s）\n", serviceName, absCfg)
	fmt.Println("现在可以在任何地方直接敲 `ms` 打开管理菜单。")
}

func cmdUninstall(_ []string) {
	if err := service.Uninstall(serviceName); err != nil {
		log.Fatalf("卸载服务失败：%v", err)
	}
	fmt.Printf("服务 %q 已移除\n", serviceName)
}

func promptString(in *bufio.Reader, label, def string) string {
	if def != "" {
		fmt.Printf("%s [%s]: ", label, def)
	} else {
		fmt.Printf("%s: ", label)
	}
	line, _ := in.ReadString('\n')
	line = strings.TrimSpace(line)
	if line == "" {
		return def
	}
	return line
}

