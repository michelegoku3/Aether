# Aether

Aether is a Windows toolkit for Steam libraries. It installs Lua scripts and
manifests, manages backups and versions, adds online/presence features, etc.

It is made of two halves:

- **AetherDesk** — the desktop app, the user-facing program.
- **AetherDLL** — C++ DLLs injected into Steam that apply the changes live.

## How to install Aether

1. Go into the Steam folder.
2. Delete everything EXCEPT `steam.exe` and the folders (you will not lose
   games, savegames or achievements).
3. Double-click `steam.exe` and wait for it to finish.
4. Download the latest AetherDesk release from
   https://github.com/michelegoku3/Aether/releases (the `desk-*` tag) and
   extract the folder on the desktop.
5. Open `AetherDesk.exe` and exclude it from your antivirus.
6. Go to the **Aether** tab and install AetherDLL.
7. Put your API key in the settings, scroll down and save.

## Discord

https://discord.gg/QdBhuYFgY
