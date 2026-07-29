!macro NSIS_HOOK_POSTINSTALL
  ; Make Windows mark AetherDesk as "Run this program as an administrator" by default.
  ; This mirrors the Compatibility-tab checkbox and applies to the installed executable.
  WriteRegStr HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe" "RUNASADMIN"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Remove the compatibility flag when AetherDesk is uninstalled.
  DeleteRegValue HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe"
!macroend
