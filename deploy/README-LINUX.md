# P2P Remote — Linux 中转服务器部署手册（Ubuntu 22.04/24.04）

正式域名：`p2p.aikissgo.com`  
协议：Caddy 终止 TLS，反代到 Docker 内网 Go WebSocket Relay（`:8080`）。  
**不是** coturn / WebRTC / ICE 打洞方案。

## 架构

```
Internet --:80/:443--> Caddy (TLS) --Docker network--> relay:8080
Windows/Android clients: wss://p2p.aikissgo.com/relay?room=<64hex>&role=host|viewer
Health: https://p2p.aikissgo.com/healthz  ->  ok
```

- Relay 访问口令：`RELAY_ACCESS_TOKEN`（Bearer），长度 ≥ 32。
- 端到端会话密钥：客户端 AES-256-GCM，**不经过**服务端明文存储。
- 局域网直连：Windows TCP `43821`，**禁止**对公网转发。

## 部署前准备（用户侧）

1. DNS：将 `p2p.aikissgo.com` 的 **A** 记录指向 VPS 公网 IPv4；若配置 **AAAA**，须保证 IPv6 实际可达。
2. 云厂商安全组：放行 TCP **80/443**；**22** 仅管理员 IP；**不要**放行 8080 / 43821。
3. 私有仓库：在 VPS 配置只读 SSH Deploy Key（或已登录的只读凭据）。**禁止**把 GitHub Token 写进脚本或仓库。
4. 本仓库 CI **不会**自动部署正式 VPS。

## 首次安装

在 VPS 上（示例）：

```bash
# 使用 Deploy Key 检出明确 commit（替换 SHA）
sudo mkdir -p /opt
sudo git clone git@github.com:chengzi1994999/p2p.git /opt/p2p-remote
cd /opt/p2p-remote
sudo git checkout --detach <COMMIT_SHA>

sudo bash deploy/install.sh --domain p2p.aikissgo.com --yes
```

或从已有工作树安装：

```bash
cd /path/to/p2p
sudo bash deploy/install.sh --domain p2p.aikissgo.com
```

脚本行为要点：

- 复用已安装的 Docker / Compose；必要时才提示安装 Docker。
- 若不存在 `deploy/.env`，生成 `0600` 文件与 `openssl rand -hex 32` Token。
- **若已有 `.env`，绝不覆盖 Token**（避免全部客户端断连）。
- `docker compose config` 通过后再 `up -d --build`。
- 备份 `compose.yml` / `Caddyfile` / `.env` 到 `deploy/backups/<timestamp>/`。
- **不会**自动改写 UFW；防火墙命令需人工确认。

### 读取 Token（安全渠道）

```bash
sudo awk -F= '/^RELAY_ACCESS_TOKEN=/{print $2}' /opt/p2p-remote/deploy/.env
```

将该 Token 与同一份 64 位十六进制 E2E 会话密钥，分别录入 Windows 与 Android。  
Token ≠ E2E 密钥。

## 客户端默认地址

| 端 | 新安装默认 | 已有本地配置 |
|----|------------|--------------|
| Windows | `wss://p2p.aikissgo.com` | 读取已保存 `settings.json`，**不会**静默覆盖 |
| Android | `wss://p2p.aikissgo.com` | SharedPreferences 已有 `relay_url` 时保留 |

UI 中仍可编辑中转地址。不要在默认 URL 后手动追加 `/relay`（客户端会自动组装）。

## 更新 / 回滚

```bash
# 更新到钉死的 SHA（禁止依赖漂浮的 latest）
sudo bash deploy/update.sh --install-dir /opt/p2p-remote --git-ref <COMMIT_SHA>

# 验收
sudo bash deploy/verify.sh --domain p2p.aikissgo.com
```

回滚：

1. `sudo bash deploy/update.sh --git-ref <旧SHA>`
2. 或从 `deploy/backups/<timestamp>/` 恢复 `compose.yml` / `Caddyfile`（`.env` 一般保持不动以保留 Token），再 `docker compose up -d --build`。
3. **不要删除** `caddy_data` / `caddy_config` 卷，以免丢失 ACME 证书状态。

## 轮换 Relay Token

1. 维护窗口通知客户端。
2. 在 VPS 生成新 Token：`openssl rand -hex 32`，写入 `deploy/.env` 的 `RELAY_ACCESS_TOKEN`（权限保持 0600）。
3. `docker compose --env-file deploy/.env -f deploy/compose.yml up -d`
4. 同步更新 Windows / Android（及可信配对资料）。旧 Token 立即失效。

## 故障排查

| 现象 | 排查 |
|------|------|
| `healthz` 超时 | DNS 是否指向本机；安全组 80/443；`docker compose ps`；`curl -v https://域名/healthz` |
| 证书失败 | DNS 未生效或 80 被拦；查看 Caddy 日志；保留 data 卷后续期 |
| 客户端 401/403 | Token 不一致或过短；是否误用 E2E 密钥当 Token |
| 能连中转无画面 | E2E 密钥不一致；Windows 未启动 Host；房间冲突（同 role 已占用） |
| 同 Wi-Fi 仍走 VPS | 路由器 AP 隔离；Windows 未勾选 LAN 43821；防火墙未放行专用网 |
| 8080 被扫到 | **错误配置**；compose 不得 `ports: 8080`；立即关闭公网 8080 |

## 安全边界（务必读）

- 共享 Bearer Token **不是**生产级多租户 / 逐设备 IAM。
- Relay 只转发密文，不落盘屏幕/剪贴板内容。
- `/healthz` 不要求鉴权，且不得返回 Token 或在线设备信息。
- `X-Forwarded-Proto` 仅在来自私网/本机的反代跳时受信任；8080 必须仅 Docker 内网可达。

## 验收清单

本地/CI：`bash -n`、`compose config`、`go test -race`、镜像构建、未授权 `/relay`→401。  
线上（需真实 VPS）：DNS、TLS、`https://p2p.aikissgo.com/healthz`、错误 Token 拒绝、LAN/WSS 切换、输入权限开关。
