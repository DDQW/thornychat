; Included near the top of Tauri's NSIS template (bundle.windows.nsis.installerHooks).
;
; ThornyChat never puts an icon on the desktop unless asked: the finish page's
; "Create desktop shortcut" box starts unticked, and a silent or passive install
; (which skips that page and would otherwise always create the icon) leaves the
; desktop alone. A shortcut that was already there is never touched.

!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

Var HadDesktopShortcut

!macro NSIS_HOOK_PREINSTALL
  StrCpy $HadDesktopShortcut 0
  IfFileExists "$DESKTOP\${PRODUCTNAME}.lnk" 0 +2
    StrCpy $HadDesktopShortcut 1
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${If} $HadDesktopShortcut = 0
      Delete "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
  ${EndIf}
!macroend
