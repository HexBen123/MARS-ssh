package main

import (
	"bufio"
	"context"
	"crypto/rand"
	"encoding/hex"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"

	"mars/internal/config"
	"mars/internal/logsink"
	"mars/internal/menu"
	"mars/internal/pubip"
	"mars/internal/relay"
	"mars/internal/service"
	"mars/internal/state"
	"mars/internal/tlsutil"
)

const logFileName = "mars-relay.log"

const (
	serviceName        = "mars-relay"
	serviceDisplayName = "MARS Reverse SSH Tunnel Relay"
	serviceDescription = "MARS (Minimal AI Reverse Ssh) 公网中转"
)

const usage = `MARS relay —— 公网侧反向隧道中转

用法：
  relay [-config <路径>]              启动（首次运行进入交互向导）
  relay run [-config <路径>]          同上
  relay ms [-config <路径>]           打开服务管理菜单
  relay install [-config <路径>]      注册为系统服务并启动
  relay uninstall                     停止服务并移除注册
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
	cfgPath := fs.String("config", "relay.yaml", "path to relay config yaml")
	_ = fs.Parse(args)

	// 首次启动：如果配置不存在，进入向导（SCM 拉起的情况跳过）
	if !config.RelayExists(*cfgPath) && !service.IsRunningAsService() {
		if err := relayWizard(*cfgPath); err != nil {
			log.Fatalf("初始化失败：%v", err)
		}
	}

	if _, err := logsink.Setup(*cfgPath, logFileName); err != nil {
		log.Printf("提示：无法打开日志文件（%v），仅输出到 stderr", err)
	}

	run := func(ctx context.Context) error {
		cfg, err := config.LoadRelay(*cfgPath)
		if err != nil {
			return err
		}
		store, err := state.Load(cfg.StateFile)
		if err != nil {
			return fmt.Errorf("load state: %w", err)
		}
		return relay.New(cfg, store).Run(ctx)
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
		log.Fatalf("relay 运行失败：%v", err)
	}
}

func relayWizard(cfgPath string) error {
	return runRelayWizard(cfgPath, nil)
}

// runRelayWizard drives the prompt flow. If `existing` is non-nil its fields
// become the defaults (used by the "修改配置" menu action; first-run passes nil).
func runRelayWizard(cfgPath string, existing *config.RelayConfig) error {
	in := bufio.NewReader(os.Stdin)
	fmt.Println("=====================================================")
	if existing == nil {
		fmt.Println(" MARS 中转 —— 首次启动向导")
	} else {
		fmt.Println(" MARS 中转 —— 修改配置")
	}
	fmt.Println(" （直接回车使用方括号里的默认值）")
	fmt.Println("=====================================================")

	defaultPort := 7000
	defaultHost := ""
	defaultMin, defaultMax := 20000, 21000
	if existing != nil {
		if p := portFromListen(existing.Listen); p != 0 {
			defaultPort = p
		}
		defaultHost = existing.PublicHost
		defaultMin = existing.PortRange.Min
		defaultMax = existing.PortRange.Max
	}

	port := promptInt(in, "控制端口（agent 用于拨入）", defaultPort, 1, 65535)

	if defaultHost == "" {
		fmt.Print("正在探测公网 IP ...  ")
		detectCtx, cancel := context.WithCancel(context.Background())
		defer cancel()
		if discovered, err := pubip.Discover(detectCtx); err == nil {
			fmt.Println(discovered)
			defaultHost = discovered
		} else {
			fmt.Printf("失败（%v）\n", err)
		}
	}
	host := promptString(in, "对外公开的域名或 IP", defaultHost)
	if host == "" {
		return fmt.Errorf("必须填写对外公开的域名或 IP")
	}

	minPort := promptInt(in, "可分配端口范围 —— 起始", defaultMin, 1024, 65535)
	maxPort := promptInt(in, "可分配端口范围 —— 结束", defaultMax, minPort, 65535)

	dir := filepath.Dir(cfgPath)
	if dir == "" {
		dir = "."
	}
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("创建目录 %s 失败：%w", dir, err)
	}

	certPath := filepath.Join(dir, "cert.pem")
	keyPath := filepath.Join(dir, "key.pem")
	statePath := filepath.Join(dir, "state.json")
	if existing != nil {
		if existing.TLS.Cert != "" {
			certPath = existing.TLS.Cert
		}
		if existing.TLS.Key != "" {
			keyPath = existing.TLS.Key
		}
		if existing.StateFile != "" {
			statePath = existing.StateFile
		}
	}

	// On first setup generate a cert; when editing an existing config keep the
	// current cert/key unless they're missing from disk (e.g. user deleted them).
	needCert := existing == nil
	if !needCert {
		if _, err := os.Stat(certPath); err != nil {
			needCert = true
		}
	}
	if needCert {
		fmt.Printf("正在生成自签证书 %s ... ", certPath)
		if err := tlsutil.GenerateSelfSigned(certPath, keyPath, []string{host}); err != nil {
			fmt.Println("失败")
			return err
		}
		fmt.Println("完成")
	}

	token := ""
	if existing != nil {
		token = existing.Token
	}
	if token == "" {
		tokenBytes := make([]byte, 32)
		if _, err := rand.Read(tokenBytes); err != nil {
			return fmt.Errorf("生成令牌失败：%w", err)
		}
		token = hex.EncodeToString(tokenBytes)
	}

	cfg := &config.RelayConfig{
		Listen:     ":" + strconv.Itoa(port),
		PublicHost: host,
		Token:      token,
		TLS:        config.TLSFiles{Cert: certPath, Key: keyPath},
		PortRange:  config.PortRange{Min: minPort, Max: maxPort},
		StateFile:  statePath,
	}
	if err := config.SaveRelay(cfgPath, cfg); err != nil {
		return fmt.Errorf("保存配置失败：%w", err)
	}

	fmt.Println()
	fmt.Println("=====================================================")
	if existing == nil {
		fmt.Println(" 配置完成。把下面两行发给目标机操作者：")
	} else {
		fmt.Println(" 配置已更新。把下面两行发给目标机操作者：")
	}
	fmt.Println("=====================================================")
	fmt.Printf("   中转地址 ： %s:%d\n", host, port)
	fmt.Printf("   令牌     ： %s\n", token)
	fmt.Println("=====================================================")
	fmt.Printf(" 配置已保存到 %s\n", cfgPath)
	if existing == nil {
		fmt.Println(" 小贴士：以后想改配置 / 启停服务，跑 `<本程序路径> ms`")
		fmt.Println("        `sudo <本程序> install` 之后，`ms` 会成为全局命令。")
		fmt.Println(" 正在启动中转 ...")
	}
	fmt.Println()
	return nil
}

func portFromListen(listen string) int {
	if listen == "" {
		return 0
	}
	s := listen
	if i := strings.LastIndex(s, ":"); i >= 0 {
		s = s[i+1:]
	}
	n, err := strconv.Atoi(s)
	if err != nil {
		return 0
	}
	return n
}

func cmdMenu(args []string) {
	fs := flag.NewFlagSet("ms", flag.ExitOnError)
	cfgPath := fs.String("config", "relay.yaml", "path to relay config yaml")
	_ = fs.Parse(args)

	absCfg, err := filepath.Abs(*cfgPath)
	if err != nil {
		log.Fatalf("解析配置路径失败：%v", err)
	}
	if !config.RelayExists(absCfg) {
		fmt.Println("还没有配置文件，先跑一次初始化向导。")
		if err := runRelayWizard(absCfg, nil); err != nil {
			log.Fatalf("初始化失败：%v", err)
		}
		fmt.Println("向导完成，现在进入管理菜单。")
	}

	bin, err := os.Executable()
	if err != nil {
		log.Fatalf("定位自身可执行文件失败：%v", err)
	}
	sp := menu.Spec{
		Title:       "MARS 中转（relay）",
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
			cur, err := config.LoadRelay(absCfg)
			if err != nil {
				return err
			}
			return runRelayWizard(absCfg, cur)
		},
		RunForeground: func() error {
			if _, err := logsink.Setup(absCfg, logFileName); err != nil {
				log.Printf("提示：无法打开日志文件（%v），仅输出到 stderr", err)
			}
			ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
			defer cancel()
			cfg, err := config.LoadRelay(absCfg)
			if err != nil {
				return err
			}
			store, err := state.Load(cfg.StateFile)
			if err != nil {
				return err
			}
			return relay.New(cfg, store).Run(ctx)
		},
	}
	if err := menu.Run(sp); err != nil {
		log.Fatalf("菜单退出：%v", err)
	}
}

func cmdInstall(args []string) {
	fs := flag.NewFlagSet("install", flag.ExitOnError)
	cfgPath := fs.String("config", "relay.yaml", "path to relay config yaml")
	_ = fs.Parse(args)

	absCfg, err := filepath.Abs(*cfgPath)
	if err != nil {
		log.Fatalf("解析配置路径失败：%v", err)
	}
	if !config.RelayExists(absCfg) {
		if err := relayWizard(absCfg); err != nil {
			log.Fatalf("初始化失败：%v", err)
		}
	}
	if _, err := config.LoadRelay(absCfg); err != nil {
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

func promptInt(in *bufio.Reader, label string, def, min, max int) int {
	for {
		s := promptString(in, label, strconv.Itoa(def))
		n, err := strconv.Atoi(s)
		if err != nil || n < min || n > max {
			fmt.Printf("  请输入 %d 到 %d 之间的整数\n", min, max)
			continue
		}
		return n
	}
}

