# Linux (WSL Ubuntu) Setup / Repair Log

Date: 2026-11-19 (UTC timestamps shown where captured)
Scope: WSL Ubuntu environment repair only. No repository source files edited; no commits made.

## 1. Initial state — VERIFIED (command output seen)

- `cat /etc/os-release` -> `PRETTY_NAME="Ubuntu 24.04.4 LTS"`, `VERSION_CODENAME=noble`, kernel `6.18.40.1-microsoft-standard-WSL2` (WSL2).
- Parent session's pre-check reported: `MOUNT=mounted`, `RUSTC=missing`, `CARGO=missing`, `GSTREAMER=missing`, `ALSA=missing` (HYPOTHESIS: state before a WSL restart; not independently re-observed by this worker).
- `dpkg --audit` (first observation, ~05:30Z) showed the expected damage from a timed-out `apt install`:
  - `libcodec2-1.2:amd64` "in a mess due to serious problems during installation", missing list + md5sums control files.
  - ~50 packages "unpacked but not yet configured" (gstreamer libs, mesa/GL/Vulkan libs, glib-networking, libavutil58, etc.).
  - Triggers pending: `libc-bin`, `libglib2.0-0t64:amd64`.
  - Warning: "Another process has locked the database for writing".
- `ps -p 368 -o ...` -> PID 368 = leftover `apt-get install -y --no-install-recommends build-essential pkg-config curl ca-certificates libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav libasound2-dev` (the timed-out install was still running).
- `/etc/wsl.conf` -> `[interop] enabled=false, appendWindowsPath=false`; `[automount] enabled=false, mountFsTab=true`.
- After a WSL restart (`uptime` -> "up 30 min" at 08:41 local), `/mnt/d` was an empty directory with no drvfs mount: `mountpoint -q /mnt/d` -> NOT_MOUNTED; `mount | grep -i drvfs` -> no output.

## 2. Commands attempted and key excerpts

1. `wsl -e bash -lc 'cat /etc/os-release; dpkg --audit | head -80; dpkg -l | grep ...'`
   -> Ubuntu 24.04.4 noble; dpkg audit showed broken/messy state (excerpt above).
2. `wsl -e bash -lc 'echo HELLO; dpkg --audit; echo AUDIT_DONE'`
   -> Full audit listing: libcodec2-1.2 in a mess (~50 unpacked-but-unconfigured packages; pending triggers libc-bin, libglib2.0-0t64).
3. `wsl -e bash -lc '... ps -p 368 ...'`
   -> PID 368 apt-get install still running (elapsed 02:59 at first check).
4. `wsl -e bash -lc 'export DEBIAN_FRONTEND=noninteractive; dpkg --configure -a; echo CONFIGURE_RC=$?'`
   -> `CONFIGURE_RC=2`; stderr: `dpkg: error: dpkg frontend lock was locked by another process with pid 368`. Decision: do NOT kill the live installer; wait for it.
5. Monitored PID 368 via `tail -N /var/log/dpkg.log`:
   - 08:30:29 `install libzmq5 ... status unpacked` (still unpacking).
   - 08:32:05 `install liblzma-dev ... status unpacked`.
   - 08:34:26 `install libqrencode4 ... status unpacked`.
   - 08:37:03 `status installed libgstreamer-plugins-base1.0-0:amd64 1.24.2-1ubuntu0.7`.
   - 08:38:46 `status installed libgstreamer1.0-dev:amd64 1.24.2-1ubuntu0.2`.
   - 08:38:49 `status installed libc-bin:amd64 2.39-0ubuntu8.12` (last entry); PID 368 subsequently `GONE`. Installer completed successfully.
6. `wsl -e bash -lc '... dpkg --audit ... pkg-config --modversion ... rustc --version ...'`
   -> `dpkg --audit` produced NO output (clean); `gstreamer-1.0` = 1.24.2; `alsa` = 1.2.11; `rustc 1.100.0 (0ef1bc386 2026-11-09)`; `cargo 1.100.0 (3acc79c0c 2026-11-05)`; `MOUNT=NOT_MOUNTED`.
   - Note: Rust was already present at this point (not installed by this worker). `which` later showed rustup-managed binaries in `/root/.cargo/bin`; exact install time/source was not observed by this worker (HYPOTHESIS: installed by a concurrent session or pre-existing rustup state).
7. `wsl -e bash -lc 'mount -t drvfs D: /mnt/d; ... mountpoint -q /mnt/d ...'`
   -> `MOUNT_RC=0`; `MOUNTED`; `ls /mnt/d` lists Windows D: contents. Manual remount succeeded.
8. `wsl -e bash -lc 'which rustc cargo rustup; rustup --version; gst-inspect-1.0 --version; gcc --version; pkg-config --version; echo $PATH'`
   -> `/root/.cargo/bin/rustc`, `/root/.cargo/bin/cargo`, `/root/.cargo/bin/rustup`; `rustup 1.29.3 (edbc831ce 2026-11-02)`; active rustc 1.100.0; `gcc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0`; `pkg-config 1.8.1`; PATH includes `/root/.cargo/bin`. `gst-inspect-1.0: command not found` at that time.
9. `wsl -e bash -lc 'export DEBIAN_FRONTEND=noninteractive; apt-get -f install -y ...; apt-get install -y --no-install-recommends gstreamer1.0-tools ...'`
   -> `apt-get -f install -y`: `0 upgraded, 0 newly installed, 0 to remove and 18 not upgraded`, `FIX_RC=0` (database consistent).
   -> `gstreamer1.0-tools (1.24.2-1ubuntu0.2)` installed successfully, `TOOLS_RC=0`.
10. Final verification at `2026-11-19T05:45:25Z`:
    -> `MOUNT_D=MOUNTED`
    -> `RUSTC=rustc 1.100.0 (0ef1bc386 2026-11-09)`
    -> `CARGO=cargo 1.100.0 (3acc79c0c 2026-11-05)`
    -> `GST_PC=1.24.2`
    -> `ALSA_PC=1.2.11`
    -> `dpkg --audit` between markers -> empty (clean).
11. `wsl -e bash -lc 'command -v gst-inspect-1.0 ...'`
    -> `/usr/bin/gst-inspect-1.0`, `WHICH_RC=0` (binary now present after installing gstreamer1.0-tools).
12. `wsl -e bash -lc 'timeout 20 gst-inspect-1.0 --version ...'`
    -> First attempt(s) returned no output within 20s / 120s tool timeouts. HYPOTHESIS at the time: first-run GStreamer plugin registry scan.
    -> Retry `wsl -e bash -lc 'echo GS_START; timeout -k 5 25 gst-inspect-1.0 --version 2>&1; echo GS_RC=$?; echo GS_END'` -> `GS_START`, `gst-inspect-1.0 version 1.24.2`, `GStreamer 1.24.2`, `https://launchpad.net/ubuntu/+source/gstreamer1.0`, `GS_RC=0`, `GS_END`. Resolved; hypothesis of slow first-run scan confirmed by successful retry.

## 3. Final status per component

| Component | Status | Evidence |
|---|---|---|
| Ubuntu version | VERIFIED | Ubuntu 24.04.4 LTS (noble), WSL2 kernel 6.18.40.1 |
| dpkg database integrity | VERIFIED | `dpkg --audit` empty; `apt-get -f install -y` RC=0, no fixes needed |
| `/mnt/d` mount | VERIFIED (manual) | `mountpoint -q /mnt/d` -> MOUNTED after `mount -t drvfs D: /mnt/d` |
| rustc | VERIFIED | `rustc 1.100.0 (0ef1bc386 2026-11-09)` -> >= 1.85, edition 2024 supported |
| cargo | VERIFIED | `cargo 1.100.0 (3acc79c0c 2026-11-05)` |
| rustup | VERIFIED | `rustup 1.29.3`, tools in `/root/.cargo/bin`, on PATH |
| gstreamer-1.0 dev (pkg-config) | VERIFIED | `pkg-config --modversion gstreamer-1.0` -> 1.24.2 |
| alsa dev (pkg-config) | VERIFIED | `pkg-config --modversion alsa` -> 1.2.11 |
| build-essential / gcc | VERIFIED | `gcc 13.3.0` |
| gstreamer plugins (base/good/bad/libav) | VERIFIED (installed by PID 368) | dpkg.log: `status installed libgstreamer-plugins-base1.0-0`, `gstreamer1.0-plugins-bad`, `libavutil58` |
| gst-inspect-1.0 binary | VERIFIED present | `command -v` -> `/usr/bin/gst-inspect-1.0` |
| gst-inspect-1.0 --version output | VERIFIED | `gst-inspect-1.0 version 1.24.2`, `GStreamer 1.24.2`, `GS_RC=0` (first run was slow; retry succeeded) |
| `cd /mnt/d/counter && cargo --version` | VERIFIED | Output: `/mnt/d/counter`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `COUNTER_OK` (completed after a delay; WSL was loaded by concurrent rustc/cargo processes from other sessions) |

## 4. Blockers / caveats

- **Automount disabled (caveat, VERIFIED config):** `/etc/wsl.conf` has `[automount] enabled = false`. After any WSL restart, `/mnt/d` will be empty until `mount -t drvfs D: /mnt/d` is run again (or the config is changed). This config was left unchanged because it appears intentional; changing it was outside the explicit scope. Intensive tests started in a fresh WSL session after a restart must remount `/mnt/d` first.
- **gst-inspect-1.0 first run (RESOLVED):** initial invocations did not return within 20s / 120s; a retry with `timeout -k 5 25` returned version 1.24.2 with RC=0. HYPOTHESIS (supported by the successful retry): one-time plugin registry scan / cold-start latency. No remaining blocker.
- **No source files changed:** no repository source edits, no commits, no Windows files modified except this log.
