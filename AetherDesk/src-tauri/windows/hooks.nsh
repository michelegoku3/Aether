; Fix v3.2 — Unificato + pulizia robusta scorciatoie + antivirus stock
; Best practices: DRY con macro/funzioni, singola responsabilità, idempotenza

!include "LogicLib.nsh"
!include "StrFunc.nsh"
${StrStr}
${StrLoc}

; Macro DRY per creare scorciatoie corrette (icona embedded, niente indice manuale)
!macro _AETHER_CREATE_SHORTCUTS
  SetShellVarContext current
  CreateShortCut "$DESKTOP\AetherDesk.lnk" "$INSTDIR\AetherDesk.exe"
  CreateDirectory "$SMPROGRAMS\AetherDesk"
  CreateShortCut "$SMPROGRAMS\AetherDesk\AetherDesk.lnk" "$INSTDIR\AetherDesk.exe"
  CreateShortCut "$SMPROGRAMS\AetherDesk\Uninstall AetherDesk.lnk" "$INSTDIR\Uninstall AetherDesk.exe"
  System::Call 'shell32::SHChangeNotify(i 0x8000000, i 0, i 0, i 0)'
!macroend

; Rimuove scorciatoie note (fisse) — veloce, copre 99% dei casi
!macro _AETHER_REMOVE_KNOWN_SHORTCUTS
  SetShellVarContext current
  Delete "$DESKTOP\AetherDesk.lnk"
  Delete "$DESKTOP\Aether.lnk"
  Delete "$DESKTOP\aether_desk.lnk"
  Delete "$SMPROGRAMS\AetherDesk\AetherDesk.lnk"
  Delete "$SMPROGRAMS\AetherDesk\Uninstall AetherDesk.lnk"
  Delete "$SMPROGRAMS\AetherDesk\aether_desk.lnk"
  RMDir "$SMPROGRAMS\AetherDesk"
  Delete "$SMPROGRAMS\Aether\AetherDesk.lnk"
  RMDir "$SMPROGRAMS\Aether"
  SetShellVarContext all
  Delete "$COMMON_DESKTOP\AetherDesk.lnk"
  Delete "$COMMON_DESKTOP\Aether.lnk"
  Delete "$COMMON_DESKTOP\aether_desk.lnk"
  Delete "$COMMON_SMPROGRAMS\AetherDesk\AetherDesk.lnk"
  Delete "$COMMON_SMPROGRAMS\AetherDesk\Uninstall AetherDesk.lnk"
  RMDir "$COMMON_SMPROGRAMS\AetherDesk"
  Delete "$COMMON_SMPROGRAMS\Aether\AetherDesk.lnk"
  RMDir "$COMMON_SMPROGRAMS\Aether"
!macroend

; Funzione robusta: scansiona Desktop e Start Menu e cancella qualsiasi .lnk
; il cui target punta a vecchio Aether (rinominata/spostata/copiata dall'utente).
; Usa ShellLink::GetShortCutTarget + FindFirst. Copre il caso "ho rinominato l'icona".
Function un.CleanStaleAetherShortcuts
  Push $R0
  Push $R1
  Push $R2
  Push $R3
  Push $R4

  ; Helper interno: pulisce una cartella (DESKTOP o SMPROGRAMS) per un contesto
  ; Viene chiamato 4 volte: current/desktop, all/desktop, current/start, all/start

  ; --- Desktop current ---
  SetShellVarContext current
  FindFirst $R0 $R1 "$DESKTOP\*.lnk"
  loop_desktop_cur:
    StrCmp $R1 "" done_desktop_cur
    ShellLink::GetShortCutTarget "$DESKTOP\$R1"
    Pop $R2
    StrCmp $R2 "" next_desktop_cur
    ${StrStr} $R2 "aether_desk.exe" $R3
    StrCmp $R3 "" +2
      Delete "$DESKTOP\$R1"
    ${StrStr} $R2 "AetherDesk.exe" $R3
    StrCmp $R3 "" +2
      ${StrStr} $R2 "Program Files" $R4
      StrCmp $R4 "" next_desktop_cur
        Delete "$DESKTOP\$R1"
    next_desktop_cur:
    FindNext $R0 $R1
    Goto loop_desktop_cur
  done_desktop_cur:
    FindClose $R0

  ; --- Desktop all ---
  SetShellVarContext all
  FindFirst $R0 $R1 "$DESKTOP\*.lnk"
  loop_desktop_all:
    StrCmp $R1 "" done_desktop_all
    ShellLink::GetShortCutTarget "$DESKTOP\$R1"
    Pop $R2
    StrCmp $R2 "" next_desktop_all
    ${StrStr} $R2 "aether_desk.exe" $R3
    StrCmp $R3 "" +2
      Delete "$DESKTOP\$R1"
    ${StrStr} $R2 "AetherDesk.exe" $R3
    StrCmp $R3 "" next_desktop_all
      ${StrStr} $R2 "Program Files" $R4
      StrCmp $R4 "" next_desktop_all
        Delete "$DESKTOP\$R1"
    next_desktop_all:
    FindNext $R0 $R1
    Goto loop_desktop_all
  done_desktop_all:
    FindClose $R0

  ; --- Start Menu current ---
  SetShellVarContext current
  FindFirst $R0 $R1 "$SMPROGRAMS\*.lnk"
  loop_sm_cur:
    StrCmp $R1 "" done_sm_cur
    ShellLink::GetShortCutTarget "$SMPROGRAMS\$R1"
    Pop $R2
    StrCmp $R2 "" next_sm_cur
    ${StrStr} $R2 "aether_desk.exe" $R3
    StrCmp $R3 "" +2
      Delete "$SMPROGRAMS\$R1"
    ${StrStr} $R2 "AetherDesk.exe" $R3
    StrCmp $R3 "" next_sm_cur
      ${StrStr} $R2 "Program Files" $R4
      StrCmp $R4 "" next_sm_cur
        Delete "$SMPROGRAMS\$R1"
    next_sm_cur:
    FindNext $R0 $R1
    Goto loop_sm_cur
  done_sm_cur:
    FindClose $R0

  ; Pulizia ricorsiva cartella AetherDesk in Start Menu (anche se utente l'ha rinominata)
  ; Scansione semplice delle sottocartelle note
  FindFirst $R0 $R1 "$SMPROGRAMS\AetherDesk\*.lnk"
  loop_sm_sub_cur:
    StrCmp $R1 "" done_sm_sub_cur
    ShellLink::GetShortCutTarget "$SMPROGRAMS\AetherDesk\$R1"
    Pop $R2
    ${StrStr} $R2 "aether_desk.exe" $R3
    StrCmp $R3 "" next_sm_sub_cur
      Delete "$SMPROGRAMS\AetherDesk\$R1"
    next_sm_sub_cur:
    FindNext $R0 $R1
    Goto loop_sm_sub_cur
  done_sm_sub_cur:
    FindClose $R0

  SetShellVarContext all
  FindFirst $R0 $R1 "$SMPROGRAMS\AetherDesk\*.lnk"
  loop_sm_sub_all:
    StrCmp $R1 "" done_sm_sub_all
    ShellLink::GetShortCutTarget "$SMPROGRAMS\AetherDesk\$R1"
    Pop $R2
    ${StrStr} $R2 "aether_desk.exe" $R3
    StrCmp $R3 "" next_sm_sub_all
      Delete "$SMPROGRAMS\AetherDesk\$R1"
    next_sm_sub_all:
    FindNext $R0 $R1
    Goto loop_sm_sub_all
  done_sm_sub_all:
    FindClose $R0

  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
  System::Call 'shell32::SHChangeNotify(i 0x8000000, i 0, i 0, i 0)'
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  !insertmacro _AETHER_REMOVE_KNOWN_SHORTCUTS
  Call un.CleanStaleAetherShortcuts
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe" "RUNASADMIN"
  CreateDirectory "$INSTDIR\AetherData\config\themes"
  CreateDirectory "$INSTDIR\AetherData\config\wallpapers"
  !insertmacro _AETHER_REMOVE_KNOWN_SHORTCUTS
  Call un.CleanStaleAetherShortcuts
  !insertmacro _AETHER_CREATE_SHORTCUTS
!macroend

; Uninstall: rispetta checkbox originale DeleteAppDataCheckboxState (1=elimina, 0=conserva)
Var AETHER_KEEP_DATA

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $AETHER_KEEP_DATA "1"
  ${If} $DeleteAppDataCheckboxState == 1
    StrCpy $AETHER_KEEP_DATA "0"
    RMDir /r "$INSTDIR\AetherData"
  ${Else}
    Rename "$INSTDIR\AetherData" "$TEMP\AetherData_keep"
  ${EndIf}
  !insertmacro _AETHER_REMOVE_KNOWN_SHORTCUTS
  Call un.CleanStaleAetherShortcuts
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe"
  ${If} $AETHER_KEEP_DATA == "1"
    CreateDirectory "$INSTDIR"
    Rename "$TEMP\AetherData_keep" "$INSTDIR\AetherData"
    IfFileExists "$INSTDIR\AetherData" +2
      CreateDirectory "$INSTDIR\AetherData"
  ${Else}
    RMDir /r "$APPDATA\com.aether.desk"
    RMDir /r "$LOCALAPPDATA\com.aether.desk"
    Delete "$TEMP\AetherData_keep"
    RMDir /r "$INSTDIR"
  ${EndIf}
  System::Call 'shell32::SHChangeNotify(i 0x8000000, i 0, i 0, i 0)'
!macroend
