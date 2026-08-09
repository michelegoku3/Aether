; Fix v3.1 — Unificato + Best Practices (09/08/2026)
; - Tutto in %LOCALAPPDATA%\AetherDesk\AetherData (currentUser, scrivibile senza admin)
; - Scorciatoie Desktop/Start Menu ricreate correttamente con icona dell'exe
; - Uninstall: rispetta la checkbox originale "Elimina dati" (DeleteAppDataCheckboxState)
;             invece di un MessageBox ridondante. Se spuntata → cancella AetherData,
;             altrimenti conserva la cartella Aether\AetherData.

; Helpers DRY per scorciatoie — alta coesione, basso accoppiamento
!macro _AETHER_CREATE_SHORTCUTS
  SetShellVarContext current
  Delete "$DESKTOP\AetherDesk.lnk"
  Delete "$DESKTOP\Aether.lnk"
  CreateShortCut "$DESKTOP\AetherDesk.lnk" "$INSTDIR\AetherDesk.exe"
  CreateDirectory "$SMPROGRAMS\AetherDesk"
  CreateShortCut "$SMPROGRAMS\AetherDesk\AetherDesk.lnk" "$INSTDIR\AetherDesk.exe"
  CreateShortCut "$SMPROGRAMS\AetherDesk\Uninstall AetherDesk.lnk" "$INSTDIR\Uninstall AetherDesk.exe"
!macroend

!macro _AETHER_REMOVE_SHORTCUTS
  SetShellVarContext current
  Delete "$DESKTOP\AetherDesk.lnk"
  Delete "$DESKTOP\Aether.lnk"
  Delete "$SMPROGRAMS\AetherDesk\AetherDesk.lnk"
  Delete "$SMPROGRAMS\AetherDesk\Uninstall AetherDesk.lnk"
  RMDir "$SMPROGRAMS\AetherDesk"
  Delete "$SMPROGRAMS\Aether\AetherDesk.lnk"
  RMDir "$SMPROGRAMS\Aether"
  SetShellVarContext all
  Delete "$COMMON_DESKTOP\AetherDesk.lnk"
  Delete "$COMMON_DESKTOP\Aether.lnk"
  Delete "$COMMON_SMPROGRAMS\AetherDesk\AetherDesk.lnk"
  Delete "$COMMON_SMPROGRAMS\AetherDesk\Uninstall AetherDesk.lnk"
  RMDir "$COMMON_SMPROGRAMS\AetherDesk"
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Pulisci scorciatoie vecchie (Program Files) prima di installare la nuova
  !insertmacro _AETHER_REMOVE_SHORTCUTS
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe" "RUNASADMIN"
  CreateDirectory "$INSTDIR\AetherData\config\themes"
  CreateDirectory "$INSTDIR\AetherData\config\wallpapers"
  ; Ricrea scorciatoie con icona corretta (usa icona embedded dell'exe, niente indice manuale)
  !insertmacro _AETHER_CREATE_SHORTCUTS
!macroend

; Uninstall: usa la checkbox originale di Tauri (DeleteAppDataCheckboxState)
; - 1 = utente ha spuntato "Elimina dati" → cancella AetherData
; - 0 = non spuntata (o update silenzioso) → conserva AetherData

Var AETHER_KEEP_DATA

!macro NSIS_HOOK_PREUNINSTALL
  ; Salva lo stato della checkbox originale per POSTUNINSTALL
  StrCpy $AETHER_KEEP_DATA "1"
  ${If} $DeleteAppDataCheckboxState == 1
    StrCpy $AETHER_KEEP_DATA "0"
    RMDir /r "$INSTDIR\AetherData"
  ${Else}
    ; Conserva: sposta fuori da INSTDIR prima che RMDir /r $INSTDIR lo cancelli
    Rename "$INSTDIR\AetherData" "$TEMP\AetherData_keep"
  ${EndIf}
  ; Rimuovi scorciatoie subito (saranno ricreate al reinstall)
  !insertmacro _AETHER_REMOVE_SHORTCUTS
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe"
  ${If} $AETHER_KEEP_DATA == "1"
    CreateDirectory "$INSTDIR"
    Rename "$TEMP\AetherData_keep" "$INSTDIR\AetherData"
    IfFileExists "$INSTDIR\AetherData" +2
      CreateDirectory "$INSTDIR\AetherData"
  ${Else}
    ; Utente ha scelto di eliminare dati: pulisci anche legacy Roaming
    RMDir /r "$APPDATA\com.aether.desk"
    RMDir /r "$LOCALAPPDATA\com.aether.desk"
    Delete "$TEMP\AetherData_keep"
    RMDir /r "$INSTDIR"
  ${EndIf}
!macroend
