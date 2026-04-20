# MARS （Minimal AI Reverse Ssh）

AI 专用的最小反向 SSH 隧道。目标服务器在内网、NAT 或没有公网 IP 时，AI
（例如 Claude Code）仍可通过公网中转机 `ssh` 进去排查。

由两个二进制组成：

- **relay** —— 部署在公网 Linux 服务器上，负责对外提供 TCP 端口
- **agent** —— 部署在 Linux/Windows 目标机上，出站连到 relay 保持长连接

## 二进制

一次编译产出所有平台的二进制，放在 `bin/` 目录：

| 文件 | 用途 | 跑在哪 |
|---|---|---|
| `bin/relay-linux-amd64` | 中转（relay） | 公网 Linux 服务器 |
| `bin/agent-linux-amd64` | 目标代理（agent） | Linux 目标机 |
| `bin/agent-linux-arm64` | 目标代理 | 树莓派 / ARM Linux |
| `bin/agent.exe` | 目标代理 | Windows 目标机 |
| `bin/relay.exe` | 中转 | 本机调试用（生产都用 Linux） |

构建（Windows 开发机）：

```bash
go build -o bin/relay.exe ./cmd/relay
go build -o bin/agent.exe ./cmd/agent
GOOS=linux   GOARCH=amd64 go build -o bin/relay-linux-amd64  ./cmd/relay
GOOS=linux   GOARCH=amd64 go build -o bin/agent-linux-amd64  ./cmd/agent
GOOS=linux   GOARCH=arm64 go build -o bin/agent-linux-arm64  ./cmd/agent
```

国内网络需先配好模块代理：

```bash
go env -w GOPROXY=https://goproxy.cn,direct
go env -w GOSUMDB=sum.golang.google.cn
```

## 三步搞定

**① 公网中转机（Linux）**：把 `relay-linux-amd64` 传上去，赋可执行权限，
直接跑，回答几个问题。

```bash
scp bin/relay-linux-amd64 user@中转机:/usr/local/bin/relay
ssh user@中转机
sudo chmod +x /usr/local/bin/relay
sudo /usr/local/bin/relay
```

向导交互示意：

```
=====================================================
 MARS 中转 —— 首次启动向导
=====================================================
控制端口（agent 用于拨入） [7000]: 7000
正在探测公网 IP ...  203.0.113.10
对外公开的域名或 IP [203.0.113.10]: relay.example.com
可分配端口范围 —— 起始 [20000]: 20000
可分配端口范围 —— 结束 [21000]: 21000
正在生成自签证书 ... 完成

=====================================================
 配置完成。把下面两行发给目标机操作者：
=====================================================
   中转地址 ： relay.example.com:7000
   令牌     ： 35bcee0b5d2b6e90ee0a12a9a713eaf7159d25e86f00f140e57b14169dc15ac9
=====================================================
```

当前目录会自动生成 `relay.yaml`、`cert.pem`、`key.pem`、`state.json`。
**把"中转地址"和"令牌"这两行发给目标机运行方。**

防火墙放行：**7000**（控制端口）和 **20000-21000**（公网 SSH 池）。

**② 目标机（Linux / Windows）**：跑 `agent`，把上一步的两行信息填进去。

Linux：

```bash
scp bin/agent-linux-amd64 user@目标机:/usr/local/bin/agent
ssh user@目标机
sudo chmod +x /usr/local/bin/agent
sudo /usr/local/bin/agent
```

Windows（管理员 PowerShell）：

```powershell
# 把 agent.exe 放到 C:\Program Files\MARS\ 下
cd 'C:\Program Files\MARS'
.\agent.exe
```

向导交互示意：

```
=====================================================
 MARS 目标代理 —— 首次启动向导
=====================================================
中转地址（host:port）: relay.example.com:7000
令牌（从中转方复制过来）: 35bcee0b5d2b...
要暴露的本地服务地址 [127.0.0.1:22]:
正在从 relay.example.com:7000 获取 TLS 指纹 ... 完成
  已钉扎：sha256:f04d6022a919...

 已注册到中转 relay.example.com:7000
 AI 或用户现在可以这样连到本机：
     ssh -p 20000 user@relay.example.com
 进来的流量会桥接到 127.0.0.1:22
```

**把底部的 `ssh -p ... user@...` 发给 AI** 就行。关键信息也会写到同目录的
`agent-info.txt`，方便以后 `cat` 查看。

**③ 注册为系统服务**（让它开机自启、断电后自动恢复）：

```bash
# Linux
sudo /usr/local/bin/agent install   # 自动写 systemd unit 并 enable --now
sudo /usr/local/bin/relay install
```

```powershell
# Windows（管理员）
C:\Program Files\MARS\agent.exe install
```

## 快捷管理菜单：`ms`

两端都支持管理菜单，用来查看状态、改配置、启停服务、设置开机自启、卸载。

**打开方式取决于是否已 `install`：**

- 还没装成服务（刚跑完向导，进程在前台）—— 在**另一个终端**里跑：
  ```bash
  ./relay-linux-amd64 ms          # 或 ./agent-linux-amd64 ms
  ```

- 已经 `install` 过 —— 直接在任何地方敲：
  ```bash
  ms
  ```
  `install` 会在 Linux 上自动创建 `/usr/local/bin/ms` 作为快捷方式（
  指向当前二进制的 `ms` 子命令 + 当前配置路径）；Windows 会在二进制
  所在目录生成 `ms.cmd`（把该目录加到 PATH 即可全局使用）。

  卸载服务（从菜单选"卸载"或跑 `<bin> uninstall`）会自动把 `ms` 快捷
  方式一并删除（带身份标记，不会误删另一端的）。

菜单示例（已注册为服务时）：

```
=====================================================
 MARS 目标代理（agent）
=====================================================
 服务名  ： mars-agent
 配置文件： /etc/mars/agent.yaml
 状态    ： 运行中 / 开机自启  (active running 12345)
-----------------------------------------------------
 1) 查看当前配置
 2) 修改配置（保存后需重启服务生效）
 3) 启动服务
 4) 停止服务
 5) 重启服务
 6) 设为开机自启
 7) 取消开机自启
 8) 卸载服务
 q) 退出
-----------------------------------------------------
选择：
```

未注册为服务时菜单更短：只有 "修改配置 / 注册为系统服务并启动 / 前台运行一次"。

顶部"状态"行会显示：是否已注册、是否在跑、是否设为开机自启，加上平台给出
的一行补充信息（systemd 的 `ActiveState SubState MainPID` 或 Windows SCM 的
`running / stopped`）。

## AI 怎么用

就是标准 SSH：

```bash
ssh -p 20000 user@relay.example.com
# scp、rsync、sshfs、ProxyCommand、-D 动态转发 全都能用
```

`~/.ssh/config`：

```
Host node-a
    HostName relay.example.com
    Port     20000
    User     ubuntu
```

## 一些细节

- **端口粘性**：同一个 agent 断线重连会拿回原来的端口。agent_id → port
  的映射持久化在中转机的 `state.json` 里，中转进程重启也不会改。
- **`ms` 修改配置**：菜单里改配置会保留 `agent_id`（目标端）和端口/证书
  （中转端），所以修改后重启服务，agent 重连仍然拿回原端口。
- **公网 IP 自动探测**：中转向导的默认值会尝试多个 echo 服务
  （`icanhazip.com`、`api.ipify.org`、`ipinfo.io`、`myip.ipip.net` 等），
  其中 `myip.ipip.net` 在国内可访问。
- **TLS 指纹钉扎**：agent 首次连接时 TOFU 拉 relay 证书指纹并存进
  `agent.yaml`。后续 TLS 握手会校验指纹，中间人挡不住。修改配置里换了
  中转地址会自动重拉指纹。
- **local_addr 在 agent 本地定死**：relay 被攻破也无法指挥 agent 拨其他
  内网地址；隧道只能到 agent 向导时填的那个地址。
- **日志文件**：relay 会把日志同时写到 stderr 和配置文件同目录下的
  `mars-relay.log`（agent 对应 `mars-agent.log`）。超过 10 MiB 会轮转
  一次到 `.log.1`，再有新日志覆盖 `.log.1`（保留最近一个备份）。

## 子命令

```
relay [run]        跑起来（首次运行进入交互向导）
relay ms           打开服务管理菜单（状态 + 改配置 + 启停 + 自启 + 卸载）
relay install      注册为系统服务并启动（Linux systemd / Windows SCM）
relay uninstall    停止并移除服务

agent [run]        同上
agent ms           同上
agent install      注册为系统服务
agent uninstall    停止并移除服务
```

默认从当前目录的 `relay.yaml` / `agent.yaml` 读配置，`-config <路径>` 可改。

## 目录

```
cmd/{relay,agent}         二进制入口、向导、ms 菜单入口
internal/config           YAML 配置与校验
internal/menu             跨平台 ms 管理菜单
internal/protocol         控制流线格式（4 字节长度前缀 + JSON）
internal/tlsutil          自签证书生成、指纹抓取与钉扎
internal/portpool         公网端口池
internal/state            agent_id → port 粘性映射（JSON 持久化）
internal/pubip            公网 IPv4 探测（国内友好）
internal/service          跨平台服务安装 / 状态 / 启停（systemd / Windows SCM）
internal/relay            中转主逻辑
internal/agent            目标代理主逻辑
```

## 运行原理

```
   AI (ssh) ──► relay:20000 ──┐
                               │  (TLS + yamux stream)
                               └─► agent ──► 127.0.0.1:22 (sshd on target)
```

- agent 出站建立一条长连的 TLS 连接到 relay 的控制端口（默认 7000）。
- yamux 在这条连接上复用多个流，每条进来的 `ssh` 会话对应一个流。
- relay 只是 TCP 桥接，不解析 SSH，不拥有 SSH 凭据；用户名/密钥认证仍然端到端。
- agent 断开：relay 立即关该 agent 的公网监听，但**保留端口号**给它重连用；
  agent 带指数退避（1s → 30s）自动重连。
