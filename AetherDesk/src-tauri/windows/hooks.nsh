!macro NSIS_HOOK_PREINSTALL
  ; Create the AetherData layout next to the executable so the folders the
  ; user expects (Settings → Appearance) already exist right after install:
  ;   <install>\AetherData\config\themes
  ;   <install>\AetherData\config\wallpapers
  ; The default Cyberpunk theme/wallpaper files are seeded by the app itself
  ; on first run (embedded in the binary, idempotent, never overwrites user
  ; files), so the installer only needs to create the directory skeleton.
  CreateDirectory "$INSTDIR\AetherData\config\themes"
  CreateDirectory "$INSTDIR\AetherData\config\wallpapers"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Make Windows mark AetherDesk as "Run this program as an administrator" by default.
  ; This mirrors the Compatibility-tab checkbox and applies to the installed executable.
  WriteRegStr HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe" "RUNASADMIN"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Remove the compatibility flag when AetherDesk is uninstalled.
  DeleteRegValue HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe"
!macroend
