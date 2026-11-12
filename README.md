# P2P Remote (个人远控测试版 V0.1)

私人使用的 **Windows 被控端 + Android 控制端** 原生远程桌面原型，源码存放在私有仓库中。

> **状态：开发原型，不是已验收的正式产品。** 首版通过 Tailscale 的加密私人网络连接，支持约 10 FPS 的 JPEG 画面、远程鼠标及文本输入；不保证锁屏、UAC、安全桌面、多屏、休眠唤醒或所有显卡可用。不含 iOS、商业会员、自建 P2P/中继服务。

## 下载测试版

进入仓库 **Actions → Build Windows and Android → 最近一次绿色成功运行 → Artifacts**，下载：

- `P2PRemote-Windows-v0.1.0-x64-test`：解压后运行 Windows `P2PHost.exe`（免安装）；
- `P2PRemote-Android-v0.1.0-debug`：Android `APK` 测试安装包。

**只有 Actions 对应任务构建为绿色，才能认为实际编译成功。** 两端需要自行完成真机和跨网络测试；debug APK 不是签名商业发行版。

## 如何连接（无需自建服务器）

1. 在 Windows 电脑及 Android 手机上分别安装并登录 [Tailscale](https://tailscale.com/)，确保处于同一 tailnet。
2. Windows 运行 `P2PHost.exe`，在界面勾选是否允许远程鼠标/键盘操作，点击 **启动远程服务**。
3. 电脑端会显示 Tailscale `100.x.x.x:22119` 地址和随机 64 位配对密钥。
4. 打开 Android App，输入电脑的 `100.x.x.x` 地址和配对密钥，点击 **连接电脑**。
5. 手机端会显示桌面；触摸/拖动可映射鼠标，输入框可将文字输入电脑（前提是 Windows 已勾选远程输入）。
6. 停止远控时在 Windows 点击 **停止远程服务** 或关闭程序。重新启动将生成新配对密钥。

**不要**在路由器上开放 22119 端口；此服务只监听 Tailscale 100.64.0.0/10 虚拟地址。

## 项目结构

```text
windows/P2PHost/           C#/.NET 8 WinForms 被控端、原生 Windows 屏幕/鼠标/键盘 API
android/                   Kotlin Android 原生 App、OkHttp WebSocket、Bitmap 解码
.github/workflows/ci.yml  自动测试版打包并上传 GitHub Actions Artifact
docs/                      安全、验收与开发路线
```

## 版本控制

- `main`：经过构建验证的主分支；
- `feature/*`：功能开发分支，通过 Pull Request 合并；
- `v0.1.0`：仅在 Windows/Android 编译和基础测试通过后创建标签；
- 正式商业版需要代码签名、许可证审计、移动应用密钥存储和人工发布审查。

## 技术及安全边界

- **仅用于你拥有或获得明确许可的电脑。** Windows 端必须有可见程序窗口，用户自主启动，且可随时停止；不做隐蔽后台常驻、权限绕过或偷偷录屏。
- 使用 Tailscale 的 WireGuard 加密连接；WebSocket 使用 `ws://` 是因为传输被限制在 tailnet 内。**禁止将服务绑定公网接口；如果以后不使用 Tailscale，必须设计端到端加密认证。**
- 采用 256-bit 随机会话密钥。密钥不会写入 GitHub、数据库或 Android 永久存储；关闭 Windows 服务后失效。
- 远程输入在 Windows 端默认关闭。第一版使用 GDI 截屏 + JPEG，不是硬件编码、高帧率或零拷贝方案；图像路径与未来 C++/Rust 编解码模块解耦。
- Windows GDI 不能可靠捕获安全桌面、UAC 提示及某些全屏图形内容；不同 Windows 会话与权限级别可能限制输入。

详细文档见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。
