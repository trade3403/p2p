using System;
using System.Drawing;
using System.Linq;
using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Windows.Forms;

namespace P2PHost;

public sealed class HostForm : Form
{
    private readonly Label _status = new() { AutoSize = true, Text = "尚未启动" };
    private readonly Label _ip = new() { AutoSize = true };
    private readonly TextBox _tokenBox = new() { ReadOnly = true, Width = 385 };
    private readonly CheckBox _allowInput = new() { Text = "允许远程鼠标和键盘操作", Checked = false, AutoSize = true };
    private readonly Button _toggle = new() { Text = "启动远程服务", AutoSize = true };
    private readonly Label _session = new() { AutoSize = true, Text = "当前无人连接" };
    private HostServer? _server;
    private volatile bool _inputAllowed;

    public HostForm()
    {
        Text = "P2P Remote - Windows 被控端 (V0.1)";
        Width = 560;
        Height = 350;
        StartPosition = FormStartPosition.CenterScreen;
        FormBorderStyle = FormBorderStyle.FixedSingle;
        MaximizeBox = false;
        var note = new Label { AutoSize = true, MaximumSize = new Size(480, 0), Text =
            "仅供本人已授权的设备使用。必须先在电脑和手机安装 Tailscale，并使用相同 tailnet。\n启动后将允许持有本次密钥的设备连接；关闭窗口即停止远程服务。" };
        var copy = new Button { Text = "复制配对密钥", AutoSize = true };
        copy.Click += (_, _) => { if (!string.IsNullOrWhiteSpace(_tokenBox.Text)) Clipboard.SetText(_tokenBox.Text); };
        var layout = new FlowLayoutPanel { Dock = DockStyle.Fill, AutoScroll = true, FlowDirection = FlowDirection.TopDown,
            WrapContents = false, Padding = new Padding(20), AutoSize = false };
        var ipTitle = new Label { AutoSize = true, Text = "电脑 Tailscale 地址（手机端填写）" };
        layout.Controls.Add(note);
        layout.Controls.Add(ipTitle);
        layout.Controls.Add(_ip);
        layout.Controls.Add(new Label { Text = "临时配对密钥（仅在本人手机输入，不要分享）", AutoSize = true });
        layout.Controls.Add(_tokenBox);
        layout.Controls.Add(copy);
        layout.Controls.Add(_allowInput);
        layout.Controls.Add(_toggle);
        layout.Controls.Add(_status);
        layout.Controls.Add(_session);
        Controls.Add(layout);
        _allowInput.CheckedChanged += (_, _) => _inputAllowed = _allowInput.Checked;
        _toggle.Click += async (_, _) =>
        {
            _toggle.Enabled = false;
            try
            {
                if (_server is null)
                {
                    var addr = FindTailnetIp();
                    if (addr is null)
                    {
                        MessageBox.Show("未找到 Tailscale IPv4 地址。请先安装并登录 Tailscale。", "无法启动");
                        return;
                    }
                    var token = Convert.ToHexString(RandomNumberGenerator.GetBytes(32)).ToLowerInvariant();
                    var server = new HostServer(addr, 22119, token, () => _inputAllowed,
                        count => { if (IsHandleCreated) BeginInvoke((Action)(() => _session.Text = count > 0 ? $"远程连接中：{count} 个会话" : "当前无人连接")); });
                    await server.StartAsync();
                    _server = server;
                    _ip.Text = addr.ToString() + ":22119";
                    _tokenBox.Text = token;
                    _status.Text = "已启动：只监听 Tailscale 私有网络";
                    _toggle.Text = "停止远程服务";
                }
                else
                {
                    await _server.StopAsync();
                    _server = null;
                    _status.Text = "已停止";
                    _session.Text = "当前无人连接";
                    _tokenBox.Clear();
                    _toggle.Text = "启动远程服务";
                }
            }
            catch (Exception ex) { MessageBox.Show(ex.Message, "服务错误"); }
            finally { _toggle.Enabled = true; }
        };
        FormClosing += (_, _) => { if (_server != null) _ = _server.StopAsync(); };
    }

    private static IPAddress? FindTailnetIp()
    {
        foreach (var nic in NetworkInterface.GetAllNetworkInterfaces())
        {
            if (nic.OperationalStatus != OperationalStatus.Up ||
                !(nic.Name.Contains("Tailscale", StringComparison.OrdinalIgnoreCase) || nic.Description.Contains("Tailscale", StringComparison.OrdinalIgnoreCase))) continue;
            foreach (var uni in nic.GetIPProperties().UnicastAddresses)
            {
                var a = uni.Address;
                if (a.AddressFamily != AddressFamily.InterNetwork) continue;
                var b = a.GetAddressBytes();
                // Tailscale allocates addresses inside 100.64.0.0/10.
                if (b[0] == 100 && b[1] is >= 64 and <= 127) return a;
            }
        }
        return null;
    }
}
