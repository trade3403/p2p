# P2P Remote 开发与验收手册

## 0. 目标与明确限制

开发对象：自己的 Windows 10/11 电脑 + Android 8.0+ 手机。第一版不部署服务器，借助用户分别安装的 Tailscale 建立安全 tailnet 路径；以后可以替换为独立的信令及 TURN 中继。

本仓库是 **V0.1 工作原型**，尚未通过真实的 GitHub 云端编译或 Android/Windows 实机测试前，不应声称能够稳定运行。

## 1. 架构

- Windows：.NET 8 WinForms 单实例 UI（手动启动/关闭）；Kestrel `WebSocket` 服务只绑定 Tailscale IPv4 地址的 `22119` 端口；Windows GDI `CopyFromScreen` 截取主显示器、等比缩放到宽度不超过 1280，并以 JPEG Quality 60 传输；Win32 `SendInput`、`SetCursorPos` 执行被明确允许的输入命令。
- Android：Kotlin `Activity` 原生 UI；OkHttp WebSocket，JPEG 使用 `BitmapFactory` 解码并显示于 `ImageView`；触摸映射至归一化鼠标坐标；附文本输入框。
- 私人网络：Tailscale 运行于两台设备上（用户本人管理）；不自建任何信令、打洞或中继服务器。Tailscale 本身使用其控制平面，无法保证完全没有第三方基础设施。
- 安全：WebSocket Header `Authorization: Bearer <64 hex chars>`；Windows 启动时产生随机 32 字节令牌，仅存内存，每次停止/启动都会轮换；所有未认证请求拒绝。只容许 1 个远控会话；远程输入默认关闭。

## 2. Windows 独立构建

```powershell
dotnet publish windows/P2PHost/P2PHost.csproj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -o dist/windows
```

## 3. Android 独立构建

需要 Android SDK 35、Java 17、Gradle 8.10.2、Android Gradle Plugin 8.7.2、Kotlin 2.1.20：

```bash
cd android
gradle --no-daemon :app:assembleDebug
```

第一版不提交 Gradle Wrapper JAR；GitHub Actions 使用 `gradle/actions/setup-gradle` 明确安装 Gradle 版本。Debug APK 用 Android Debug 密钥签名；**它不是可用于商业上架的发布签名包**。

## 4. Actions 版本管理及自动构建

1. 私有仓库 PR 和主分支 push 均触发 `.github/workflows/ci.yml`。
2. Windows Runner 发布独立 `P2PHost.exe` 并压缩为 ZIP；Android Runner 打包 `app-debug.apk`。
3. Upload Artifacts 默认保留 14 天。这个流程**没有**自动部署到真实设备，也没有安装软件，更不代表通过实机稳定性验收。
4. Git 分支：`feature/*` → PR → 审阅/CI → `main`。只有两端构建和测试都通过，才创建 `v0.x.y` 标记。
5. 正式版需新增 GitHub Environment、审核机制、证书签名、Release 正式包、SBOM 和安全扫描。

## 5. 可执行验收清单

- [ ] PR 构建 Windows Job 成功，Artifacts 里有 ZIP、SHA256。
- [ ] PR 构建 Android Job 成功，Artifacts 里有 APK、SHA256。
- [ ] Windows 11 安装 Tailscale，运行 EXE，看到状态和临时密钥。
- [ ] 同一个家庭 Wi-Fi：Android 能看到 Windows 主桌面。
- [ ] Android 使用蜂窝 4G/5G：通过 Tailscale 连接同一台电脑。
- [ ] 不允许输入时，点击与文本指令不会操作 Windows。
- [ ] 手动允许输入后，点击、拖动和文字输入可用。
- [ ] 错误令牌无法连接，连接时不接受第二个远控客户端。
- [ ] Windows 停止服务后远控连接关闭；重新启动密钥改变。
- [ ] 网络暂时断开后可手动重连，8 小时测试不崩溃（以后增加自动重连）。
- [ ] 在 Windows 屏幕休眠、锁屏、UAC 及多显卡场景分别记录行为与限制，不绕过系统权限。

## 6. V0.2 后续任务

1. 分离系统采集、编码、网络协议与 UI 接口；改为 Windows Graphics Capture / DXGI + H.264 硬件编码。
2. Android MediaCodec 硬件解码与低延迟缓冲、屏幕旋转优化、双指滚动与键盘快捷键。
3. 首次配对授权、Android Keystore 保存受保护的设备凭证、令牌轮换和会话撤销。
4. 断线自动重连、心跳、网络质量评估、帧率/码率自适应。
5. Windows 计划任务或用户可见服务形式的明确授权无人值守；不做隐藏持续控制。
6. 独立的本地连接发现与未来自建 P2P/中继适配层，脱离对 Tailscale 的产品依赖。
7. 扩展 CI：静态扫描、单元/集成测试、版本签名、人工审批发布。
