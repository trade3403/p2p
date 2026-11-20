Unicode true
!include "MUI2.nsh"

!define APP_NAME "P2P Remote"
!define APP_VERSION "0.19.0"
!define APP_PUBLISHER "P2P Remote"
!define APP_EXE "p2p_remote_ui.exe"
!define APP_SOURCE_DIR "..\dist\v0.19-windows"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\P2PRemote"

Name "${APP_NAME} ${APP_VERSION}"
OutFile "..\P2PRemote-Setup-v${APP_VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\P2P Remote"
RequestExecutionLevel user
ShowInstDetails show
ShowUnInstDetails show
VIProductVersion "0.19.0.0"
VIAddVersionKey /LANG=2052 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=2052 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey /LANG=2052 "FileDescription" "P2P Remote Desktop Client"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "立即启动 P2P Remote"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Section "P2P Remote" SEC_MAIN
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File /r "${APP_SOURCE_DIR}\*.*"

  IfFileExists "$INSTDIR\${APP_EXE}" +2 0
    Abort "安装包中缺少主程序 ${APP_EXE}。请重新下载完整安装包。"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\P2P Remote"
  CreateShortcut "$SMPROGRAMS\P2P Remote\P2P Remote.lnk" "$INSTDIR\${APP_EXE}"
  CreateShortcut "$SMPROGRAMS\P2P Remote\卸载 P2P Remote.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\P2P Remote.lnk" "$INSTDIR\${APP_EXE}"

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$DESKTOP\P2P Remote.lnk"
  Delete "$SMPROGRAMS\P2P Remote\P2P Remote.lnk"
  Delete "$SMPROGRAMS\P2P Remote\卸载 P2P Remote.lnk"
  RMDir "$SMPROGRAMS\P2P Remote"
  RMDir /r "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ; DPAPI credentials in the current user's profile are intentionally preserved.
SectionEnd
