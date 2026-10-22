# Testing the AetherDesk self-updater

**What it is:** how to exercise the portable AetherDesk self-updater — the file
swap on its own, and a full update through a test-channel release — without
shipping a real release.

**Read it when:** you change the updater, the release discovery or the
`--apply-update` helper, or you want to verify the update path before publishing
a build.

**Not here:** the build gates and how to run them (see
[`docs/quality-gates.md`](quality-gates.md)) or the on-disk layout both halves
share (see [`docs/shared_contracts.md`](shared_contracts.md)).

## How the updater works

AetherDesk ships as a portable ZIP: no installer, no registry, no Start Menu
entry. The updater in `AetherDesk/src-tauri/src/updater/desk.rs` therefore
downloads a portable ZIP from a GitHub release, extracts it and swaps the files
in place; `AetherDesk/src-tauri/src/updater/github.rs` decides which release is
offered.

Two streams exist, and they are independent:

| Stream | Tags | Offered when |
|---|---|---|
| Stable | `desk-*`, `desk-v*` | the tag version is newer than the installed version |
| Test | `tdesk-*`, `tdesk-v*` | "Enable test updates" is on **and** the tag version is newer than the installed version |

- Drafts are never considered, on either stream. The release API accepts
  prereleases on the test stream and rejects them on the stable one: a
  `tdesk-*` release may be a prerelease, a `desk-*` release must not be.
- The version gate is the same on both streams (`latest_is_newer_than`): the
  numeric parts of the tag version are compared with the installed version, and
  only a strictly greater version is an update. An equal or older tag is
  reported as up to date.
- With test updates on, an existing `tdesk-*` release is the answer even when it
  is not newer: the check then reports up to date and does not fall back to
  stable. It falls back only when the test channel is empty or its lookup fails.
- The asset must be named `AetherDesk-<version>.zip`, where `<version>` is the
  tag without the component prefix and without the optional `v` (`tdesk-v1.4.4`
  → `AetherDesk-1.4.4.zip`). The lookup prefers a `.zip` whose name contains
  `aetherdesk` or `desk`, and otherwise takes any `.zip` asset of the release.

The swap cannot run in the process it replaces, because Windows locks a running
`.exe`:

1. `install_aether_desk_update` downloads the ZIP and extracts it into staging;
2. `schedule_restart` copies the running `AetherDesk.exe` into a per-run
   directory and launches the copy with `--apply-update <staging> <install_root>`;
3. the app exits, and the copy waits for the original exe to unlock, renames it
   to `AetherDesk.exe.old`, merges the staging tree into the install folder
   while skipping `AetherData`, deletes staging, removes `.old` and relaunches
   `AetherDesk.exe`.

`run_apply_update` (`AetherDesk/src-tauri/src/main.rs`) refuses an install root
that is not absolute, that contains a link or reparse point, that is exactly one
of the protected user folders (home, Desktop, Downloads, Documents, AppData
roaming/local, config), or whose `AetherDesk.exe` is not byte-identical
(SHA-256) to the running helper. The hash rule is why a manual test must run a
copy of the *installed* exe, not the exe you just built.

Artifacts live under `<install_root>/AetherData/temp/desk_update`: the staged
ZIP, the `staging/` folder, and one `run-<pid>-<n>/` directory per attempt
holding the helper copy. The next startup deletes the ZIP and `staging/`; a run
directory stays while its helper is still running and is deleted on a later
startup once it is an hour old (`cleanup_stale_artifacts`).

There are two independent ways to test it: the swap half alone against a
throwaway install, with no GitHub and no release; and the whole flow on the test
channel, from a built ZIP to a published prerelease.

## Test the swap locally (no GitHub)

This exercises everything after extraction: root validation, the wait for the
old exe, the rename, the merge, the cleanup and the relaunch. It needs a
portable build from `build_desk.cmd` and the release exe that build left in the
Cargo target folder; the variables below point at both.

```powershell
$src   = "C:\Users\Roberto\Desktop\Generale\Codice\Aether\AetherDesk\build\portable\AetherDesk"
$built = "C:\Users\Roberto\Desktop\Generale\Codice\Aether\AetherDesk\src-tauri\target\release\AetherDesk.exe"
$test  = "C:\Users\Roberto\Desktop\updater-test"

# 1. A throwaway install: the portable folder plus user data.
Remove-Item -Recurse -Force "$test\install" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$test\install" | Out-Null
Copy-Item -Recurse -Force "$src\*" "$test\install"
New-Item -ItemType Directory -Force "$test\install\AetherData" | Out-Null
Set-Content "$test\install\AetherData\keep.txt" "user data must survive the swap"

# 2. Staging: the freshly built exe plus a file only the merge can bring in.
Remove-Item -Recurse -Force "$test\staging" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$test\staging" | Out-Null
Copy-Item -Force $built "$test\staging\AetherDesk.exe"
Set-Content "$test\staging\swap-marker.txt" "staged by the updater test"

# 3. Run the swap from a copy of the installed exe.
Copy-Item -Force "$test\install\AetherDesk.exe" "$env:TEMP\aether_upd.exe"
& "$env:TEMP\aether_upd.exe" --apply-update "$test\staging" "$test\install"
"exit code: $LASTEXITCODE"
```

Both the app and its copy carry the `requireAdministrator` manifest, so
PowerShell asks for elevation before the swap runs, exactly as the real update
does.

Check the result:

```powershell
Test-Path "$test\staging"                                                    # False: staging removed
Test-Path "$test\install\swap-marker.txt"                                    # True: the merge ran
Get-Content "$test\install\AetherData\keep.txt"                              # user data untouched
Test-Path "$test\install\AetherDesk.exe.old"                                 # False
(Get-FileHash "$test\install\AetherDesk.exe").Hash -eq (Get-FileHash $built).Hash   # True: exe replaced
Get-Process AetherDesk | Select-Object Id, Path                              # the relaunched app
```

The relaunched window belongs to the throwaway install. Close it before running
the test again: while it holds the install exe, the next helper waits 30 seconds
for the lock and then fails with `timed out waiting for the old exe to unlock`.

A refusal is equally cheap to check — point the helper at a protected folder and
read its exit code:

```powershell
& "$env:TEMP\aether_upd.exe" --apply-update "$test\staging" "$env:USERPROFILE\Desktop"
"exit code: $LASTEXITCODE"   # 1
```

The helper prints `[AetherDesk] refusing update:` followed by the validation
error (`Refusing protected uninstall root ...`; the validator is shared with the
portable uninstaller, hence the word "uninstall").

What this covers: the apply half end to end, including the `AetherData` skip and
the relaunch. What it does not cover: GitHub release discovery, the version gate,
the ZIP download, extraction (which locates the app root inside the archive),
`schedule_restart` and the helper staging it performs.

## Test the whole flow on the test channel

1. Bump the version in all three files, to the same number:
   - `AetherDesk/src-tauri/tauri.conf.json`, field `version`;
   - `AetherDesk/package.json`, field `version`;
   - `AetherDesk/src-tauri/Cargo.toml`, `[package] version`.

   The installed version the updater compares against is the one Tauri reads
   from `AetherDesk/src-tauri/tauri.conf.json`. `check:tauri-versions`
   (`AetherDesk/scripts/check_tauri_versions.mjs`, part of the verify chain)
   compares the Tauri runtime, JS API and CLI lines, not this app version, so a
   bump that misses a file still builds.

2. Build from the repository root with `build_desk.cmd`. It writes the portable
   folder and the ZIP beside it, named after `tauri.conf.json`:
   `AetherDesk/build/portable/AetherDesk-<version>.zip`.

3. Publish the prerelease. The tag carries the test prefix; the asset keeps the
   name the build gave it:

   ```powershell
   $v = "1.4.4"
   gh release create "tdesk-$v" `
     "C:\Users\Roberto\Desktop\Generale\Codice\Aether\AetherDesk\build\portable\AetherDesk-$v.zip" `
     --prerelease --title "AetherDesk test $v" --notes "Updater test build"
   ```

4. Turn on Settings → Aether → "Enable test updates" (the `enable_test_updates`
   field in `AetherDesk/src-tauri/src/core/settings_model.rs`).

5. Run the check. It runs at startup, so open the app (or restart it) and look
   at the Aether view: the AetherDesk "Update" button carries the test dot.
   Clicking it downloads the ZIP, exits the app, lets the helper swap the files
   and relaunches the new build. Verify the version label and that `AetherData`
   kept its contents.

6. Delete the release when you are done:

   ```powershell
   gh release delete "tdesk-$v" --yes
   ```

The version gate applies here exactly as it does for stable: a tag that is not
newer than the installed build is reported as up to date (`... is not newer than
installed version ...` in the log) and nothing is offered. Rebuilding the same
version produces a ZIP the updater ignores, so the bump in step 1 is what makes
the release an update. Stable users never see these releases: the stable lookup
filters by the `desk-` prefix, and the `tdesk-` probe only runs with test updates
enabled.

## Troubleshooting

Logs are in the app's Log view, or in `AetherData/logs/desk.log` under the
install folder.

| What you see | Cause |
|---|---|
| `refusing update: ... Refusing protected uninstall root ...` | The install root is exactly home, Desktop, Downloads, Documents, AppData roaming/local, or config. Point it at a folder of its own. |
| `refusing update: ... AetherDesk.exe does not match this helper in ...` | The helper is not a copy of the exe in the install folder: the freshly built exe was run directly, or the install was rebuilt after the copy. |
| `refusing update: ... requires an absolute install path`, `Refusing linked/reparse path` | A relative path, or a junction/symlink anywhere in the path chain. |
| `AetherDesk test release tdesk-x is not newer than installed version y` | The version was not bumped, or was bumped only in `package.json`/`Cargo.toml` while `tauri.conf.json` stayed behind. |
| `Could not find a portable .zip asset in AetherDesk release tdesk-x` | The release has no `.zip`, or the asset was renamed while uploading. |
| `No tdesk-* release published`, or the check answers with the stable channel | The release is a draft, or the tag lacks the `tdesk-` prefix. A `desk-*` release marked prerelease is invisible on the stable stream as well. |
| A test build reaches stable users, or stays invisible with test updates on | The tag uses the prefix of the other stream. `tdesk-*` is probed only with test updates enabled; `desk-*` is the stable channel. Publish under the prefix of the stream you mean to test. |
| `tdesk-* channel known empty (checked recently)` | The app remembers an empty test channel for fifteen minutes. Restart it to probe again. |
| `Release lookup ... served from cache (origin=..., age_ms=...)` | Lookups are reused for a minute and an empty channel for half an hour. Wait, or restart the app. |
| `AetherDesk update check failed: GitHub API failed ... and public fallback failed ...` | No outbound HTTPS or DNS. Rule connectivity out first: the hubcap provider logs `API key validation network failure` when its key check cannot reach the server and `Manifest ZIP body read failed` when a download body ends early — the same class of failure that stops a release lookup or a ZIP download. |
| `AetherDesk.exe.old` left in the install folder | The apply process stopped between renaming the exe and cleaning up, or putting the old exe back failed. A failed merge also prints `update apply failed: ...` on stderr; that line and the failing path are visible when the helper runs by hand as in the local test above. |

## Related

- [`docs/quality-gates.md`](quality-gates.md) — what every build check blocks.
- [`docs/shared_contracts.md`](shared_contracts.md) — the on-disk layout,
  including `AetherData`.
- [`docs/INDEX.md`](INDEX.md) — every document, with "read it when".
- [`AGENTS.md`](../AGENTS.md#build-test-lint) — build commands.
