using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Net;
using System.Net.WebSockets;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Windows.Forms;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;

namespace P2PHost;

public sealed class HostServer
{
    private readonly IPAddress _bind;
    private readonly int _port;
    private readonly string _token;
    private readonly Func<bool> _canInput;
    private readonly Action<int> _onSessions;
    private readonly SemaphoreSlim _oneSession = new(1, 1);
    private WebApplication? _app;

    public HostServer(IPAddress bind, int port, string token, Func<bool> canInput, Action<int> onSessions)
    {
        _bind = bind;
        _port = port;
        _token = token;
        _canInput = canInput;
        _onSessions = onSessions;
    }

    public async Task StartAsync()
    {
        var builder = WebApplication.CreateBuilder();
        builder.WebHost.ConfigureKestrel(options => options.Listen(_bind, _port));
        var app = builder.Build();
        app.UseWebSockets(new WebSocketOptions { KeepAliveInterval = TimeSpan.FromSeconds(20) });
        app.MapGet("/", () => Results.Text("P2P Host ready (Tailscale only)"));
        app.Map("/ws", HandleClientAsync);
        await app.StartAsync();
        _app = app;
    }

    public async Task StopAsync()
    {
        if (_app is not { } app) return;
        _app = null;
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(3));
        await app.StopAsync(timeout.Token);
        await app.DisposeAsync();
    }

    private async Task HandleClientAsync(HttpContext context)
    {
        var got = context.Request.Headers.Authorization.ToString();
        const string prefix = "Bearer ";
        var received = got.StartsWith(prefix, StringComparison.Ordinal) ? got[prefix.Length..] : string.Empty;
        var expectedBytes = Encoding.UTF8.GetBytes(_token);
        var receivedBytes = Encoding.UTF8.GetBytes(received);
        if (receivedBytes.Length != expectedBytes.Length || !CryptographicOperations.FixedTimeEquals(receivedBytes, expectedBytes))
        {
            context.Response.StatusCode = StatusCodes.Status401Unauthorized;
            return;
        }
        if (!context.WebSockets.IsWebSocketRequest)
        {
            context.Response.StatusCode = StatusCodes.Status400BadRequest;
            return;
        }
        if (!await _oneSession.WaitAsync(0))
        {
            context.Response.StatusCode = StatusCodes.Status409Conflict;
            return;
        }

        try
        {
            using var ws = await context.WebSockets.AcceptWebSocketAsync();
            _onSessions(1);
            using var stop = CancellationTokenSource.CreateLinkedTokenSource(context.RequestAborted);
            var transmit = SendFramesAsync(ws, stop.Token);
            var commands = ReadCommandsAsync(ws, stop.Token);
            await Task.WhenAny(transmit, commands);
            stop.Cancel();
            try { await Task.WhenAll(transmit, commands); }
            catch (Exception e) when (e is OperationCanceledException or WebSocketException or IOException or ObjectDisposedException) { }
            if (ws.State == WebSocketState.Open)
            {
                try { await ws.CloseAsync(WebSocketCloseStatus.NormalClosure, "Disconnected", CancellationToken.None); }
                catch (WebSocketException) { }
            }
        }
        catch (Exception e) when (e is OperationCanceledException or WebSocketException or IOException) { }
        finally { _onSessions(0); _oneSession.Release(); }
    }

    private static async Task SendFramesAsync(WebSocket ws, CancellationToken cancel)
    {
        while (!cancel.IsCancellationRequested && ws.State == WebSocketState.Open)
        {
            var frame = CaptureJpeg();
            await ws.SendAsync(new ArraySegment<byte>(frame), WebSocketMessageType.Binary, true, cancel);
            await Task.Delay(100, cancel); // MVP: 10 FPS, optimize with dirty-region and hardware encoding later.
        }
    }

    private static byte[] CaptureJpeg()
    {
        var bounds = Screen.PrimaryScreen?.Bounds ?? throw new InvalidOperationException("No active display");
        using var original = new Bitmap(bounds.Width, bounds.Height, PixelFormat.Format24bppRgb);
        using (var g = Graphics.FromImage(original)) g.CopyFromScreen(bounds.Left, bounds.Top, 0, 0, bounds.Size);
        var w = Math.Min(1280, bounds.Width);
        var h = Math.Max(1, (int)Math.Round(bounds.Height * w / (double)bounds.Width));
        using var scaled = new Bitmap(w, h, PixelFormat.Format24bppRgb);
        using (var g = Graphics.FromImage(scaled))
        {
            g.InterpolationMode = InterpolationMode.Bilinear;
            g.DrawImage(original, new Rectangle(0, 0, w, h));
        }
        using var memory = new MemoryStream();
        var encoder = ImageCodecInfo.GetImageEncoders().First(x => x.FormatID == ImageFormat.Jpeg.Guid);
        using var settings = new EncoderParameters(1);
        settings.Param[0] = new EncoderParameter(System.Drawing.Imaging.Encoder.Quality, 60L);
        scaled.Save(memory, encoder, settings);
        return memory.ToArray();
    }

    private async Task ReadCommandsAsync(WebSocket ws, CancellationToken cancel)
    {
        var bytes = new byte[4096];
        while (!cancel.IsCancellationRequested && ws.State == WebSocketState.Open)
        {
            var result = await ws.ReceiveAsync(new ArraySegment<byte>(bytes), cancel);
            if (result.MessageType == WebSocketMessageType.Close) return;
            if (result.MessageType != WebSocketMessageType.Text || !result.EndOfMessage) continue;
            if (!_canInput()) continue;
            try
            {
                using var message = JsonDocument.Parse(bytes.AsMemory(0, result.Count));
                var command = message.RootElement;
                var type = command.GetProperty("type").GetString();
                if (type == "text")
                {
                    var text = command.GetProperty("text").GetString() ?? "";
                    if (text.Length <= 500) InputControl.SendText(text);
                    continue;
                }
                if (type is not ("down" or "up" or "move" or "scroll")) continue;
                var x = command.GetProperty("x").GetDouble();
                var y = command.GetProperty("y").GetDouble();
                if (!double.IsFinite(x) || !double.IsFinite(y) || x < 0 || x > 1 || y < 0 || y > 1) continue;
                InputControl.Apply(type, x, y);
            }
            catch (Exception e) when (e is JsonException or InvalidOperationException or KeyNotFoundException or FormatException) { }
        }
    }
}

internal static class InputControl
{
    [StructLayout(LayoutKind.Sequential)] private struct MOUSEINPUT { public int dx; public int dy; public uint mouseData; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] private struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Explicit)] private struct InputUnion { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] private struct INPUT { public uint type; public InputUnion data; }
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError = true)] private static extern uint SendInput(uint count, INPUT[] inputs, int size);

    private static void Mouse(uint flag, uint data = 0)
    {
        var input = new INPUT { type = 0, data = new InputUnion { mi = new MOUSEINPUT { dwFlags = flag, mouseData = data } } };
        SendInput(1, new[] { input }, Marshal.SizeOf<INPUT>());
    }

    public static void Apply(string command, double x, double y)
    {
        var bounds = Screen.PrimaryScreen?.Bounds;
        if (bounds is null) return;
        SetCursorPos(bounds.Value.Left + (int)(x * (bounds.Value.Width - 1)), bounds.Value.Top + (int)(y * (bounds.Value.Height - 1)));
        switch (command)
        {
            case "down": Mouse(0x0002); break; // LEFTDOWN
            case "up": Mouse(0x0004); break; // LEFTUP
            case "scroll": Mouse(0x0800, 120); break;
        }
    }

    public static void SendText(string text)
    {
        foreach (var ch in text)
        {
            var down = new INPUT { type = 1, data = new InputUnion { ki = new KEYBDINPUT { wScan = ch, dwFlags = 0x0004 } } };
            var up = new INPUT { type = 1, data = new InputUnion { ki = new KEYBDINPUT { wScan = ch, dwFlags = 0x0004 | 0x0002 } } };
            SendInput(2, new[] { down, up }, Marshal.SizeOf<INPUT>());
        }
    }
}
