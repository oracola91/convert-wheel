; Pirouette was called Convert Wheel on Windows up to 0.3.x. The installer under the
; new name puts the app in a new folder (%LOCALAPPDATA%\Pirouette) with a new
; uninstall entry, so the copy under the old name is uninstalled first: one app, one
; Start menu entry, one entry in Installed apps. Settings and the log stay, as they
; live under the app id (local.convertwheel). The old uninstaller also takes away the
; Start menu and desktop shortcuts and the Open at Login value, so those that were
; there come back under the new name. The MSI keeps the old upgrade code
; (tauri.windows.conf.json), so an MSI update replaces the old MSI copy by itself.

!define OLDNAME "Convert Wheel"
!define OLDUNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${OLDNAME}"
!define RUNKEY "Software\Microsoft\Windows\CurrentVersion\Run"

Var OldRemoved
Var OldRunAtLogin
Var OldDesktopShortcut

!macro NSIS_HOOK_PREINSTALL
  StrCpy $OldRemoved 0
  StrCpy $OldRunAtLogin ""
  StrCpy $OldDesktopShortcut 0
  ; Where the old copy is and its uninstaller (quoted), as the old installer saved them.
  ; MANUKEY only inside the macros: the installer defines it after reading this file.
  ReadRegStr $R8 HKCU "${MANUKEY}\${OLDNAME}" ""
  ReadRegStr $R9 HKCU "${OLDUNINSTKEY}" "UninstallString"
  ${If} $R8 != ""
  ${AndIf} $R9 != ""
  ${AndIf} $R8 != $INSTDIR
  ${AndIf} ${FileExists} "$R8\uninstall.exe"
    ReadRegStr $OldRunAtLogin HKCU "${RUNKEY}" "${OLDNAME}"
    ${If} ${FileExists} "$DESKTOP\${OLDNAME}.lnk"
      StrCpy $OldDesktopShortcut 1
    ${EndIf}
    DetailPrint "Removing ${OLDNAME}, this app under its old name"
    ; Silent: it closes the old copy if it still runs. _?= makes it run in place and
    ; wait, so it can't delete itself or its folder: that is done after it.
    ExecWait '$R9 /S _?=$R8' $0
    DetailPrint "Old uninstaller finished ($0)"
    Delete "$R8\uninstall.exe"
    RMDir "$R8"
    DeleteRegKey HKCU "${MANUKEY}\${OLDNAME}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
    StrCpy $OldRemoved 1
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $OldRemoved = 1
    ; An update (/UPDATE) makes no shortcuts, as it expects the old ones to be there.
    ${IfNot} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${EndIf}
    ${If} $OldDesktopShortcut = 1
    ${AndIfNot} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
      CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
    ${If} $OldRunAtLogin != ""
      ; The same value the app writes (login.rs): this copy, quoted.
      WriteRegStr HKCU "${RUNKEY}" "${PRODUCTNAME}" '"$INSTDIR\${MAINBINARYNAME}.exe"'
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; The app makes its Send To entry itself (send_to.rs); an uninstall takes it away,
  ; an update leaves it.
  ${If} $UpdateMode <> 1
    Delete "$APPDATA\Microsoft\Windows\SendTo\${PRODUCTNAME}.lnk"
    ; And the link its notifications carry (win_toast.rs).
    DeleteRegKey HKCU "Software\Classes\pirouette"
  ${EndIf}
!macroend
