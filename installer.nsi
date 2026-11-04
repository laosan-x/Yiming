; CocoleIced NSIS Installer Script
; Usage: pwsh -File scripts/build-installer.ps1
;        (reads Cargo.toml and passes -DAPP_VERSION; a bare makensis run
;         only produces a clearly-marked 0.0.0-dev sentinel build)

!define APP_NAME "CocoleIced"
!define APP_NAME_CN "YiMing Product Matting Workbench"

; Version: MUST be supplied via -DAPP_VERSION (injected from the tag in CI,
; read from Cargo.toml by scripts/build-installer.ps1 locally). The sentinel
; default below ensures a bare `makensis installer.nsi` can never silently
; produce a package that looks like a real release.
; NOTE: keep this file ASCII-only. NSIS 3.x reads scripts using the system
; ANSI codepage (ACP) and rejects BOM-less UTF-8 non-ASCII text.
!ifndef APP_VERSION
    !define APP_VERSION "0.0.0-dev"
    !warning "APP_VERSION not set: using 0.0.0-dev sentinel. Use scripts/build-installer.ps1"
!endif
!define APP_PUBLISHER "YiMing"
!define APP_URL "https://github.com/laosan-x/Yiming"
!define APP_EXE "cocole-iced.exe"

; Install directory (default Program Files)
!define INSTALL_DIR "$PROGRAMFILES64\CocoleIced"

; Basic settings
Name "${APP_NAME} v${APP_VERSION}"
OutFile "dist\cocole-iced-v${APP_VERSION}-setup.exe"
InstallDir "${INSTALL_DIR}"
InstallDirRegKey HKLM "Software\${APP_NAME}" "InstallDir"
RequestExecutionLevel admin
Icon "icons\app.ico"
SetCompressor /SOLID lzma

; Include Modern UI
!include "MUI2.nsh"

; MUI pages
!define MUI_ABORTWARNING
!define MUI_ICON "icons\app.ico"
!define MUI_UNICON "icons\app.ico"

; Welcome page
!insertmacro MUI_PAGE_WELCOME
; Directory page
!insertmacro MUI_PAGE_DIRECTORY
; Install files page
!insertmacro MUI_PAGE_INSTFILES
; Finish page
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Run ${APP_NAME}"
!insertmacro MUI_PAGE_FINISH

; Uninstall confirm page
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; Language
!insertmacro MUI_LANGUAGE "English"

; Install section
Section "Main Program" SecMain
    SectionIn RO
    SetOutPath "$INSTDIR"

    ; Write install info to registry
    WriteRegStr HKLM "Software\${APP_NAME}" "InstallDir" "$INSTDIR"
    WriteRegStr HKLM "Software\${APP_NAME}" "Version" "${APP_VERSION}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "DisplayName" "${APP_NAME_CN}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "Publisher" "${APP_PUBLISHER}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "DisplayIcon" "$INSTDIR\${APP_EXE}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "NoModify" 1
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}" \
        "NoRepair" 1

    ; Copy main executable
    File "dist\cocole-iced-v${APP_VERSION}-windows-x64\${APP_EXE}"

    ; Copy icons
    SetOutPath "$INSTDIR\icons"
    File "dist\cocole-iced-v${APP_VERSION}-windows-x64\icons\*.*"

    ; Copy ONNX Runtime DLLs
    SetOutPath "$INSTDIR\ort-libs"
    File "dist\cocole-iced-v${APP_VERSION}-windows-x64\ort-libs\*.*"

    ; Copy AI models
    SetOutPath "$INSTDIR\models"
    File "dist\cocole-iced-v${APP_VERSION}-windows-x64\models\*.*"

    ; Copy example image
    SetOutPath "$INSTDIR\tools"
    File "tools\Example.jpg"

    ; Create Start Menu shortcuts
    CreateDirectory "$SMPROGRAMS\${APP_NAME}"
    CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
    CreateShortCut "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk" "$INSTDIR\uninstall.exe"

    ; Create Desktop shortcut
    CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

    ; Write uninstaller
    WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

; Uninstall section
Section "Uninstall"
    ; Keep user-downloaded large model weights; only remove the 3 bundled lightweight models
    Delete "$INSTDIR\models\face_detection_yunet_2023mar.onnx"
    Delete "$INSTDIR\models\modnet.onnx"
    Delete "$INSTDIR\models\u2netp.onnx"
    RMDir "$INSTDIR\models"          ; Kept if the dir is not empty (user models present)

    ; Delete remaining install files
    RMDir /r "$INSTDIR\icons"
    RMDir /r "$INSTDIR\ort-libs"
    RMDir /r "$INSTDIR\tools"
    Delete "$INSTDIR\${APP_EXE}"
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"

    ; Delete Start Menu
    RMDir /r "$SMPROGRAMS\${APP_NAME}"

    ; Delete Desktop shortcut
    Delete "$DESKTOP\${APP_NAME}.lnk"

    ; Delete registry keys
    DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"
    DeleteRegKey HKLM "Software\${APP_NAME}"
SectionEnd