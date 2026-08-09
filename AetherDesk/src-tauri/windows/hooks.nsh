; Fix v3 (09/08/2026) — Tutto unificato in %LOCALAPPDATA%\AetherDesk\AetherData
; Requisito: l'intera cartella di Aether deve stare insieme.
; - InstallMode=currentUser → $INSTDIR è %LOCALAPPDATA%\AetherDesk (scrivibile senza admin)
; - Dati in $INSTDIR\AetherData (themes, wallpapers, settings, backup)
; - Disinstallazione senza spunta dati → rimuove tutto ECCETTO $INSTDIR\AetherData
; - Disinstallazione con spunta dati → rimuove tutto da %LOCALAPPDATA% (incluso AetherData)

!macro NSIS_HOOK_PREINSTALL
  ; Non serve creare AetherData qui: viene creata a runtime e la migrazione
  ; da Roaming/Program Files la popola se necessario.
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe" "RUNASADMIN"
  ; Assicura che AetherData esista già dopo install (vuota, poi seed a primo avvio)
  CreateDirectory "$INSTDIR\AetherData\config\themes"
  CreateDirectory "$INSTDIR\AetherData\config\wallpapers"
!macroend

; --- Uninstall: gestisce l'opzione "Elimina anche i dati" ---
; Tauri NSIS non espone di default una checkbox "delete data", quindi usiamo
; un MessageBox YES/NO nel pre-uninstall per chiedere all'utente.
; Se l'utente sceglie NO (conserva dati), spostiamo AetherData fuori da $INSTDIR
; prima che l'uninstaller faccia RMDir /r $INSTDIR, poi lo ripristiniamo.

Var AETHER_KEEP_DATA

!macro NSIS_HOOK_PREUNINSTALL
  MessageBox MB_YESNO|MB_ICONQUESTION "Vuoi eliminare anche i dati utente?$\n$\nSì = elimina TUTTO da %LOCALAPPDATA%\AetherDesk (incluso AetherData con temi/wallpaper)$\nNo = conserva la cartella Aether\AetherData" IDYES delete_data IDNO keep_data
  delete_data:
    StrCpy $AETHER_KEEP_DATA "0"
    ; Rimuovi subito AetherData così RMDir /r la elimina
    RMDir /r "$INSTDIR\AetherData"
    Goto done
  keep_data:
    StrCpy $AETHER_KEEP_DATA "1"
    ; Sposta AetherData fuori da INSTDIR per proteggerla
    Rename "$INSTDIR\AetherData" "$TEMP\AetherData_keep"
  done:
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\AetherDesk.exe"
  StrCmp $AETHER_KEEP_DATA "1" restore_data no_restore
  restore_data:
    ; L'uninstaller ha già fatto RMDir /r $INSTDIR, quindi ricrea la cartella Aether con AetherData
    CreateDirectory "$INSTDIR"
    Rename "$TEMP\AetherData_keep" "$INSTDIR\AetherData"
    ; Se per qualche motivo il Rename fallisce (es. $TEMP pulito), prova a non perdere dati da Roaming legacy
    IfFileExists "$INSTDIR\AetherData" +2
      CreateDirectory "$INSTDIR\AetherData"
    Goto end
  no_restore:
    ; Utente ha scelto di eliminare tutto: assicurati che $INSTDIR sia rimosso anche se era stato ricreato
    RMDir /r "$INSTDIR"
    ; Pulisci anche eventuale legacy Roaming se esiste ancora
    RMDir /r "$APPDATA\com.aether.desk"
  end:
!macroend
