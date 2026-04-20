// Package menu renders the shared MARS management menu used by both the
// relay and agent binaries when the user runs the `ms` subcommand.
package menu

import (
	"bufio"
	"fmt"
	"os"
	"regexp"
	"strings"

	"mars/internal/service"
)

// Spec describes one end of MARS for the menu.
type Spec struct {
	Title        string // shown at the top, e.g. "MARS 中转（relay）"
	ServiceName  string // e.g. "mars-relay"
	ConfigPath   string // absolute path to the YAML on disk
	Binary       string // absolute path to the running binary (for install)
	InstallSpec  service.Spec
	EditConfig   func() error   // called when user picks "修改配置"
	RunForeground func() error  // start the service in foreground (current process)
}

// Run shows the menu loop. Returns when the user picks "退出".
func Run(sp Spec) error {
	in := bufio.NewReader(os.Stdin)
	for {
		st, _ := service.QueryStatus(sp.ServiceName)
		renderHeader(sp, st)

		var options []option
		if st.Installed {
			options = installedOptions(sp)
		} else {
			options = notInstalledOptions(sp)
		}

		for _, o := range options {
			fmt.Printf(" %s) %s\n", o.key, o.label)
		}
		fmt.Println(" q) 退出")
		fmt.Println("-----------------------------------------------------")
		fmt.Print("选择： ")

		line, err := in.ReadString('\n')
		if err != nil {
			return nil
		}
		choice := strings.TrimSpace(line)
		if choice == "q" || choice == "Q" || choice == "" {
			return nil
		}

		handled := false
		for _, o := range options {
			if o.key == choice {
				if err := o.action(); err != nil {
					fmt.Printf("\n!! 操作失败：%v\n", err)
				} else if o.successMsg != "" {
					fmt.Printf("\n✓ %s\n", o.successMsg)
				}
				handled = true
				break
			}
		}
		if !handled {
			fmt.Printf("\n!! 无效选项：%q\n", choice)
		}
		fmt.Println()
		fmt.Print("按回车继续 ...")
		_, _ = in.ReadString('\n')
	}
}

type option struct {
	key        string
	label      string
	action     func() error
	successMsg string
}

func renderHeader(sp Spec, st service.Status) {
	clearScreen()
	fmt.Println("=====================================================")
	fmt.Printf(" %s\n", sp.Title)
	fmt.Println("=====================================================")
	fmt.Printf(" 服务名  ： %s\n", sp.ServiceName)
	fmt.Printf(" 配置文件： %s\n", sp.ConfigPath)
	fmt.Printf(" 状态    ： %s\n", renderStatus(st))
	fmt.Println("-----------------------------------------------------")
}

func renderStatus(st service.Status) string {
	if !st.Installed {
		return "未安装为系统服务"
	}
	parts := []string{}
	if st.Running {
		parts = append(parts, "运行中")
	} else {
		parts = append(parts, "已停止")
	}
	if st.Enabled {
		parts = append(parts, "开机自启")
	} else {
		parts = append(parts, "未设为自启")
	}
	label := strings.Join(parts, " / ")
	if st.Detail != "" {
		label += "  (" + st.Detail + ")"
	}
	return label
}

func installedOptions(sp Spec) []option {
	return []option{
		{
			key:    "1",
			label:  "查看当前配置",
			action: func() error { return viewConfig(sp.ConfigPath) },
		},
		{
			key:        "2",
			label:      "修改配置（保存后需重启服务生效）",
			action:     sp.EditConfig,
			successMsg: "配置已保存",
		},
		{
			key:        "3",
			label:      "启动服务",
			action:     func() error { return service.Start(sp.ServiceName) },
			successMsg: "已发送启动指令",
		},
		{
			key:        "4",
			label:      "停止服务",
			action:     func() error { return service.Stop(sp.ServiceName) },
			successMsg: "已停止",
		},
		{
			key:        "5",
			label:      "重启服务",
			action:     func() error { return service.Restart(sp.ServiceName) },
			successMsg: "已重启",
		},
		{
			key:        "6",
			label:      "设为开机自启",
			action:     func() error { return service.Enable(sp.ServiceName) },
			successMsg: "已设为开机自启",
		},
		{
			key:        "7",
			label:      "取消开机自启",
			action:     func() error { return service.Disable(sp.ServiceName) },
			successMsg: "已取消开机自启",
		},
		{
			key:        "8",
			label:      "卸载服务",
			action:     func() error { return service.Uninstall(sp.ServiceName) },
			successMsg: "已卸载",
		},
	}
}

func notInstalledOptions(sp Spec) []option {
	opts := []option{
		{
			key:    "1",
			label:  "查看当前配置",
			action: func() error { return viewConfig(sp.ConfigPath) },
		},
		{
			key:        "2",
			label:      "修改配置",
			action:     sp.EditConfig,
			successMsg: "配置已保存",
		},
		{
			key:        "3",
			label:      "注册为系统服务并启动",
			action:     func() error { return service.Install(sp.InstallSpec) },
			successMsg: "已注册并启动",
		},
	}
	if sp.RunForeground != nil {
		opts = append(opts, option{
			key:    "4",
			label:  "前台运行一次（Ctrl+C 停止）",
			action: sp.RunForeground,
		})
	}
	return opts
}

// tokenLine matches a YAML line like `token: abcdef...` so we can redact the
// long secret when showing the config on screen.
var tokenLine = regexp.MustCompile(`(?m)^(\s*token\s*:\s*)(\S+)`)

func viewConfig(path string) error {
	if path == "" {
		return fmt.Errorf("配置路径为空")
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	redacted := tokenLine.ReplaceAllStringFunc(string(b), func(m string) string {
		parts := tokenLine.FindStringSubmatch(m)
		prefix, val := parts[1], parts[2]
		if len(val) <= 12 {
			return prefix + val
		}
		return prefix + val[:8] + "...（已省略）"
	})
	fmt.Println()
	fmt.Println("-----------------------------------------------------")
	fmt.Printf(" 配置文件：%s\n", path)
	fmt.Println("-----------------------------------------------------")
	fmt.Print(redacted)
	if !strings.HasSuffix(redacted, "\n") {
		fmt.Println()
	}
	fmt.Println("-----------------------------------------------------")
	return nil
}

func clearScreen() {
	// ANSI: clear + home cursor. Works in Windows Terminal, cmd.exe on Win10+,
	// all modern *nix terminals. Worst case you see the escape once — harmless.
	fmt.Print("\x1b[2J\x1b[H")
}
