/*
 * com0com-pro installer — migrated from com0com 3.0.0 NSIS/install.nsi
 * and modernized for the com0com-pro toolchain (ccp / ccp-gui / ccp-service).
 *
 * Build:
 *   makensis installer\com0com-pro.nsi        (from the com0com-pro root)
 *   Output: installer\com0com-pro-setup.exe
 *
 * Layout expectations (relative to com0com-pro root):
 *   ..\com0com\x64\com0com.sys / .cat / *.inf / setup.dll / setupc.exe
 *   target\release\ccp.exe / ccp-gui.exe / ccp-service.exe
 */

  Unicode true
  !include "MUI2.nsh"
  !include "x64.nsh"
  !include "FileFunc.nsh"

;--------------------------------

  !define PRODUCT_NAME "com0com-pro"
  !define PRODUCT_VERSION "0.3.0"
  !define PRODUCT_PUBLISHER "com0com-pro project"
  !define PRODUCT_WEB_SITE "https://github.com/wensincai"

;--------------------------------

; The name of the installer
Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"

; The file to write
OutFile "com0com-pro-setup.exe"

; The default installation directory
InstallDir $PROGRAMFILES64\com0com-pro

; Registry key to check for directory (re-install overwrites old dir)
InstallDirRegKey HKLM "Software\com0com-pro" "Install_Dir"

; Installer touches driver + services
RequestExecutionLevel admin

ShowInstDetails show
ShowUninstDetails show

;--------------------------------
; Pages

  !define MUI_WELCOMEPAGE_TITLE_3LINES
  !define MUI_FINISHPAGE_TITLE_3LINES

  !define MUI_FINISHPAGE_RUN
  !define MUI_FINISHPAGE_RUN_TEXT "Launch com0com-pro GUI"
  !define MUI_FINISHPAGE_RUN_FUNCTION LaunchGUI
  !define MUI_FINISHPAGE_RUN_NOTCHECKED

  !define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\ReadMe.txt"
  !define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

  !insertmacro MUI_PAGE_WELCOME
  !insertmacro MUI_PAGE_LICENSE "..\LICENSE"
  !insertmacro MUI_PAGE_COMPONENTS
  !insertmacro MUI_PAGE_DIRECTORY
  !insertmacro MUI_PAGE_INSTFILES
  !insertmacro MUI_PAGE_FINISH

  !define MUI_UNFINISHPAGE_NOAUTOCLOSE
  !insertmacro MUI_UNPAGE_WELCOME
  !insertmacro MUI_UNPAGE_CONFIRM
  !insertmacro MUI_UNPAGE_INSTFILES
  !insertmacro MUI_UNPAGE_FINISH

;--------------------------------
; Languages

  !insertmacro MUI_LANGUAGE "English"
  !insertmacro MUI_LANGUAGE "SimpChinese"

;--------------------------------

Function LaunchGUI
  Exec '"$INSTDIR\ccp-gui.exe"'
FunctionEnd

;--------------------------------

!macro RunSetupc args
  Push $0
  GetTempFileName $0
  StrCpy $1 ""
  IfSilent 0 +2
    StrCpy $1 "--silent"
  ExecWait '"$INSTDIR\setupc.exe" $1 --output "$0" ${args}'
  Pop $0
!macroend

;--------------------------------

Section "-tools" sec_tools

  SectionIn RO

  ; Application binaries
  SetOutPath "$INSTDIR"
  File "..\target\release\ccp.exe"
  File "..\target\release\ccp-gui.exe"
  File "..\target\release\ccp-service.exe"
  File "..\README.md"
  File "..\LICENSE"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Registry: install dir + uninstall entries
  WriteRegStr HKLM "Software\com0com-pro" "Install_Dir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "DisplayName" "${PRODUCT_NAME} ${PRODUCT_VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "URLInfoAbout" "${PRODUCT_WEB_SITE}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "DisplayIcon" "$INSTDIR\ccp-gui.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "InstallLocation" "$INSTDIR\"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" "NoRepair" 1

  ${GetSize} "$INSTDIR" "/S=0B" $R0 $R1 $R2
  IntOp $R0 $R0 / 1024
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro" \
    "EstimatedSize" $R0

SectionEnd

;--------------------------------

Section "com0com driver (x64)" sec_driver

  ; Driver files, migrated from com0com x64 build
  SetOutPath "$INSTDIR"
  File "..\..\com0com\x64\com0com.sys"
  File /nonfatal "..\..\com0com\x64\com0com.cat"
  File "..\..\com0com\x64\com0com.inf"
  File "..\..\com0com\x64\cncport.inf"
  File "..\..\com0com\x64\comport.inf"
  File "..\..\com0com\x64\setup.dll"
  File "..\..\com0com\x64\setupc.exe"
  File "..\..\com0com\x64\test-sign\com0com-test.cer"

  ; Trust the test-signing certificate: without this, SetupCopyOEMInf fails
  ; with 0x800B0109 (untrusted root) and UpdateDriverForPlugAndPlayDevices
  ; fails with error 265 (untrusted publisher).
  nsExec::ExecToLog 'certutil -f -addstore Root "$INSTDIR\com0com-test.cer"'
  Pop $0
  nsExec::ExecToLog 'certutil -f -addstore TrustedPublisher "$INSTDIR\com0com-test.cer"'
  Pop $0

  ; Driver preinstall / update / inf cleanup (same as original installer)
  !insertmacro RunSetupc "preinstall"
  !insertmacro RunSetupc "update"
  !insertmacro RunSetupc "infclean"

SectionEnd

;--------------------------------

Section "Start Menu Shortcuts" sec_shortcuts

  CreateDirectory "$SMPROGRAMS\com0com-pro"
  CreateShortCut "$SMPROGRAMS\com0com-pro\com0com-pro GUI.lnk" "$INSTDIR\ccp-gui.exe"
  CreateShortCut "$SMPROGRAMS\com0com-pro\ccp Command Prompt.lnk" "$INSTDIR\ccp.exe"
  CreateShortCut "$SMPROGRAMS\com0com-pro\ReadMe.lnk" "$INSTDIR\README.md"
  CreateShortCut "$SMPROGRAMS\com0com-pro\Uninstall.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\uninstall.exe" 0

SectionEnd

;--------------------------------

Section "Register bundle service" sec_service

  ; Installs + starts the ccp-service (bundles rebuilt at boot)
  nsExec::ExecToLog '"$INSTDIR\ccp.exe" service install --exe "$INSTDIR\ccp-service.exe"'
  Pop $0

SectionEnd

;--------------------------------

Section "Create port pair CNCA0 <-> CNCB0" sec_pair

  !insertmacro RunSetupc "install 0 - -"

SectionEnd

;--------------------------------

Function .onInit

  ; x64-only: this package ships the amd64 driver
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP \
      "This package includes only the 64-bit (amd64) driver.$\n$\n32-bit systems are not supported by this installer."
    Abort
  ${EndIf}

  ; Default selections: shortcuts on, service off, pair off
  SectionGetFlags ${sec_service} $0
  IntOp $1 ${SF_SELECTED} ~
  IntOp $0 $0 & $1
  SectionSetFlags ${sec_service} $0
  SectionGetFlags ${sec_pair} $0
  IntOp $0 $0 & $1
  SectionSetFlags ${sec_pair} $0

FunctionEnd

;--------------------------------

; Uninstaller

Section "Uninstall"

  SetOutPath "$INSTDIR"

  ; Stop & remove bundle service first (it may hold no driver handles,
  ; but closing cleanly is the right order anyway)
  nsExec::ExecToLog '"$INSTDIR\ccp.exe" service uninstall'
  Pop $0

  ; Remove ports + driver devices (setupc uninstall)
  !insertmacro RunSetupc "uninstall"

  ; Remove registry keys
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\com0com-pro"
  DeleteRegKey HKLM "Software\com0com-pro"

  ; Remove files
  Delete "$INSTDIR\ccp.exe"
  Delete "$INSTDIR\ccp-gui.exe"
  Delete "$INSTDIR\ccp-service.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\com0com.sys"
  Delete "$INSTDIR\com0com.cat"
  Delete "$INSTDIR\com0com.inf"
  Delete "$INSTDIR\cncport.inf"
  Delete "$INSTDIR\comport.inf"
  Delete "$INSTDIR\setup.dll"
  Delete "$INSTDIR\setupc.exe"
  Delete "$INSTDIR\uninstall.exe"

  ; Remove shortcuts
  Delete "$SMPROGRAMS\com0com-pro\*.*"
  RMDir "$SMPROGRAMS\com0com-pro"

  SetOutPath $TEMP
  RMDir "$INSTDIR"

SectionEnd

;--------------------------------

  ; Language strings
  LangString DESC_sec_tools ${LANG_ENGLISH} "Install ccp.exe / ccp-gui.exe / ccp-service.exe (required)."
  LangString DESC_sec_driver ${LANG_ENGLISH} "Install the com0com kernel driver (com0com.sys + co-installers)."
  LangString DESC_sec_shortcuts ${LANG_ENGLISH} "Add shortcuts to the Start Menu."
  LangString DESC_sec_service ${LANG_ENGLISH} "Register ccp-service so bundles are rebuilt automatically at boot."
  LangString DESC_sec_pair ${LANG_ENGLISH} "Create a default linked port pair CNCA0 <-> CNCB0."

  LangString DESC_sec_tools ${LANG_SIMPCHINESE} "安装 ccp.exe / ccp-gui.exe / ccp-service.exe（必选）。"
  LangString DESC_sec_driver ${LANG_SIMPCHINESE} "安装 com0com 内核驱动（com0com.sys 及安装辅助）。"
  LangString DESC_sec_shortcuts ${LANG_SIMPCHINESE} "添加开始菜单快捷方式。"
  LangString DESC_sec_service ${LANG_SIMPCHINESE} "注册 ccp-service，开机自动重建 bundle 配置。"
  LangString DESC_sec_pair ${LANG_SIMPCHINESE} "创建默认互联端口对 CNCA0 <-> CNCB0。"

  !insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
    !insertmacro MUI_DESCRIPTION_TEXT ${sec_tools} $(DESC_sec_tools)
    !insertmacro MUI_DESCRIPTION_TEXT ${sec_driver} $(DESC_sec_driver)
    !insertmacro MUI_DESCRIPTION_TEXT ${sec_shortcuts} $(DESC_sec_shortcuts)
    !insertmacro MUI_DESCRIPTION_TEXT ${sec_service} $(DESC_sec_service)
    !insertmacro MUI_DESCRIPTION_TEXT ${sec_pair} $(DESC_sec_pair)
  !insertmacro MUI_FUNCTION_DESCRIPTION_END

;--------------------------------
