; The app had another name and program file up to 0.3.x (src-tauri/src/migrate.rs).
; A copy under the earlier name lives in another folder with another uninstall entry,
; so it is uninstalled first: one app, one Start menu entry, one entry in Installed
; apps. Its uninstaller also takes away the Start menu and desktop shortcuts and the
; Open at Login value, so those that were there come back under the new name. An
; update of a copy that still had the earlier program file leaves that file and
; shortcuts pointing at it: the file goes and the shortcuts are made again. The app
; itself moves the settings on its first start (migrate.rs). The MSI keeps the old
; upgrade code (tauri.windows.conf.json), so an MSI update replaces the old MSI copy.

!define OLDNAME "Convert Wheel"
!define OLDPROGRAM "convert-wheel"
; Where the installer under the earlier app id kept its own keys: Tauri names that folder
; after the publisher, which was the app id's second part then. Now it is "Pirouette".
!define OLDMANUKEY "Software\convertwheel"
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
  ReadRegStr $R8 HKCU "${OLDMANUKEY}\${OLDNAME}" ""
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
    DeleteRegKey HKCU "${OLDMANUKEY}\${OLDNAME}"
    DeleteRegKey /ifempty HKCU "${OLDMANUKEY}"
    StrCpy $OldRemoved 1
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; A copy installed under the earlier app id kept its keys in the old publisher folder.
  DeleteRegKey HKCU "${OLDMANUKEY}\${PRODUCTNAME}"
  DeleteRegKey /ifempty HKCU "${OLDMANUKEY}"
  ${If} ${FileExists} "$INSTDIR\${OLDPROGRAM}.exe"
    Delete "$INSTDIR\${OLDPROGRAM}.exe"
    ${If} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${EndIf}
    ${If} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
      CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
  ${EndIf}
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
