$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$runner = Join-Path $repoRoot 'flutter_app\windows\runner'
$headerPath = Join-Path $runner 'flutter_window.h'
$sourcePath = Join-Path $runner 'flutter_window.cpp'
$cmakePath = Join-Path $runner 'CMakeLists.txt'
foreach ($path in @($headerPath, $sourcePath, $cmakePath)) {
    if (-not (Test-Path $path)) { throw "Flutter Windows runner file is missing: $path" }
}

$nl = [Environment]::NewLine
$header = Get-Content -LiteralPath $headerPath -Raw
if ($header -notmatch 'kTrayCallbackMessage') {
    $header = $header.Replace('#include <windows.h>', '#include <windows.h>' + $nl + '#include <shellapi.h>')
    $declarations = @'
  static constexpr UINT kTrayCallbackMessage = WM_APP + 0x41;
  static constexpr UINT kTrayMenuShow = 41001;
  static constexpr UINT kTrayMenuExit = 41002;
  NOTIFYICONDATAW tray_icon_data_{};
  bool tray_icon_added_ = false;
  void AddTrayIcon();
  void RemoveTrayIcon();
  void ShowFromTray();
  void ShowTrayMenu();
'@
    $header = [regex]::Replace($header, '(\sprivate:\s*)', ('$1' + $declarations + $nl), 1)
    Set-Content -LiteralPath $headerPath -Value $header -Encoding UTF8
}

$source = Get-Content -LiteralPath $sourcePath -Raw
if ($source -notmatch 'void FlutterWindow::AddTrayIcon') {
    $include = '#include "flutter_window.h"' + $nl +
        '#include <shellapi.h>' + $nl +
        '#include <windows.h>' + $nl +
        '#include "resource.h"'
    $source = $source.Replace('#include "flutter_window.h"', $include)
    $methods = @'
void FlutterWindow::AddTrayIcon() {
  tray_icon_data_ = {};
  tray_icon_data_.cbSize = sizeof(NOTIFYICONDATAW);
  tray_icon_data_.hWnd = GetHandle();
  tray_icon_data_.uID = 1;
  tray_icon_data_.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
  tray_icon_data_.uCallbackMessage = kTrayCallbackMessage;
  tray_icon_data_.hIcon =
      LoadIcon(GetModuleHandle(nullptr), MAKEINTRESOURCE(IDI_APP_ICON));
  if (tray_icon_data_.hIcon == nullptr) {
    tray_icon_data_.hIcon = LoadIcon(nullptr, IDI_APPLICATION);
  }
  wcscpy_s(tray_icon_data_.szTip, L"P2P Remote");
  tray_icon_added_ = Shell_NotifyIconW(NIM_ADD, &tray_icon_data_) == TRUE;
}

void FlutterWindow::RemoveTrayIcon() {
  if (tray_icon_added_) {
    Shell_NotifyIconW(NIM_DELETE, &tray_icon_data_);
    tray_icon_added_ = false;
  }
}

void FlutterWindow::ShowFromTray() {
  ShowWindow(GetHandle(), SW_RESTORE);
  SetForegroundWindow(GetHandle());
}

void FlutterWindow::ShowTrayMenu() {
  HMENU menu = CreatePopupMenu();
  if (menu == nullptr) return;
  AppendMenuW(menu, MF_STRING, kTrayMenuShow, L"显示主界面");
  AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(menu, MF_STRING, kTrayMenuExit, L"退出 P2P Remote");
  POINT point{};
  GetCursorPos(&point);
  SetForegroundWindow(GetHandle());
  TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, point.x, point.y, 0,
                 GetHandle(), nullptr);
  DestroyMenu(menu);
}

'@
    $source = $source -replace 'bool FlutterWindow::OnCreate\(\)', ($methods + 'bool FlutterWindow::OnCreate()')
    $source = $source -replace '(SetChildContent\(flutter_controller_->view\(\)->GetNativeWindow\(\)\);)', ('$1' + $nl + '  AddTrayIcon();')
    $source = $source -replace '(void FlutterWindow::OnDestroy\(\) \{)', ('$1' + $nl + '  RemoveTrayIcon();')
    $messagePattern = 'LRESULT FlutterWindow::MessageHandler\([^)]*\)\s*noexcept\s*\{'
    if (-not [regex]::IsMatch($source, $messagePattern)) {
        throw 'Could not locate FlutterWindow::MessageHandler for tray message handling.'
    }
    $messageHandling = @'
LRESULT FlutterWindow::MessageHandler(HWND window, UINT const message,
                                      WPARAM const wparam,
                                      LPARAM const lparam) noexcept {
  if (message == WM_CLOSE && tray_icon_added_) {
    ShowWindow(window, SW_HIDE);
    return 0;
  }
  if (message == kTrayCallbackMessage) {
    if (lparam == WM_LBUTTONUP || lparam == WM_LBUTTONDBLCLK) {
      ShowFromTray();
    } else if (lparam == WM_RBUTTONUP || lparam == WM_CONTEXTMENU) {
      ShowTrayMenu();
    }
    return 0;
  }
  if (message == WM_COMMAND) {
    switch (LOWORD(wparam)) {
      case kTrayMenuShow:
        ShowFromTray();
        return 0;
      case kTrayMenuExit:
        DestroyWindow(window);
        return 0;
    }
  }
'@
    $source = [regex]::Replace($source, $messagePattern, [System.Text.RegularExpressions.MatchEvaluator]{ param($match) $messageHandling }, 1)
    Set-Content -LiteralPath $sourcePath -Value $source -Encoding UTF8
}

$cmake = Get-Content -LiteralPath $cmakePath -Raw
if ($cmake -notmatch 'shell32') {
    $binaryName = '$' + '{BINARY_NAME}'
    Add-Content -LiteralPath $cmakePath -Value ($nl + 'target_link_libraries(' + $binaryName + ' PRIVATE shell32)' + $nl)
}

Write-Host 'Windows runner tray integration installed: close hides, tray click restores, context menu exits.'
