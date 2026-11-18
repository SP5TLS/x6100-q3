# x6100-q3

An SD card Linux system for the Xiegu X6100 transceiver that turns it into
a small Quake III Arena machine: Linux and U-Boot from mainline sources,
built with Buildroot, ioquake3 on the radio's Mali-400 GPU, and a start
screen (`x6100-launcher`) with which you set up Wi-Fi and Bluetooth,
upload the game data from a browser and start the game.

It contains no game data and no radio software. You need your own copy
of Quake III Arena (see [Game data](#game-data)).

| | |
|---|---|
| Buildroot | 2026.02.3 (LTS), downloaded and checksum-verified by `make` |
| Kernel | Linux 6.18.58 (LTS), `sunxi_defconfig` + a fragment, three patches |
| Boot loader | U-Boot 2026.01, standard boot from `extlinux.conf` |
| Graphics | Mesa 26 `lima` (Mali-400 MP2), desktop GL 2.1 over EGL/GBM, SDL2 on KMS/DRM |
| Game | ioquake3 (OpenGL1 renderer), patched to turn the picture for the sideways panel |
| Start screen | `x6100-launcher` (Rust, no dependencies): menu on the screen, web page on port 80 |
| Network | NetworkManager, avahi (`x6100.local`), chrony, OpenSSH |
| Bluetooth | BlueZ 5, `x6100-btd` pairing agent; keyboards, mice, headphones (A2DP, SBC) |
| Audio | PipeWire + WirePlumber: Bluetooth headphones or speakers, USB sound cards |

## Building

On Linux with [Buildroot's host packages](https://buildroot.org/downloads/manual/manual.html#requirement):

```sh
make                 # first run: fetch Buildroot, configure, build (an hour or two)
make x6100-launcher-rebuild all   # after changing launcher/
make flash DEV=/dev/sdX
```

On macOS, or anywhere without those packages, the same in Docker:

```sh
scripts/docker-build.sh          # -> images/sdcard.img
scripts/docker-build.sh x6100-launcher-rebuild all   # any make targets, in order
scripts/docker-build.sh --dist   # image for others, see below
```

Write `images/sdcard.img` (or `output/images/sdcard.img`) to a microSD
card with `dd`, balenaEtcher or Raspberry Pi Imager and put it in the
radio. The X6100 starts from the card when one is inserted; the stock
system on the internal eMMC is not touched. Take the card out to get the
stock radio back.

The container is pinned (`docker/Dockerfile`): Debian by dated tag and
digest, and its packages from snapshot.debian.org at the time that image
was made, so a new Debian release or package update cannot change or
break the build. Everything Buildroot downloads is checked against
hashes. To move to a newer Debian, change the tag, digest and snapshot
time there together.

The launcher can also be built and tried on its own: `cd launcher &&
cargo test`. `cargo build` runs on a desktop too, with `X6100_ROOT`
pointing at a directory that stands in for `/` and `--fb` at a file
(see `launcher/src/main.rs`).

### Your own card and images for others

The gitignored `secrets/` directory holds things that go into **your**
image only: Wi-Fi networks (`*.nmconnection`), SSH keys
(`authorized_keys`), and files preloaded into the data partition
(`data/`, for example `data/quake3/baseq3/pak0.pk3`; the partition is
made big enough). Such an image contains your passwords and possibly
your game data: do not share it.

`scripts/docker-build.sh --dist` makes an image you may give to others:
a clean build of the committed sources only (a `git archive` of `HEAD`:
nothing from `secrets/`), plus Buildroot's legal-info. You get
`images/x6100-q3-<version>.img.xz` and
`images/x6100-q3-<version>-legal-info.tar`. **Publish the legal-info
archive together with the image:** it holds the licence texts and the
complete source code of every package in it, which the GPL and LGPL
require you to offer along with the binaries. The licence texts and the
package list are also in the image (`/usr/share/legal-info`, and on the
web page under *Licences*).

### GitHub Actions

`.github/workflows/image.yml` runs on pushes to `main`, pull requests,
tags and by hand (*Run workflow*):

1. **checks**: the launcher's tests and clippy with Rust 1.88 (the version
   Buildroot builds it with) and `reuse lint`.
2. **image**: `scripts/docker-build.sh --dist` on a hosted runner (1.5 to
   3 hours; Buildroot's downloads are cached between runs). The image and
   its legal-info archive are uploaded as the run's artifacts (kept 14
   days). The two belong together: whoever gets the image must be able to
   get the sources too.
3. **release**: for a tag `v*`, both are published as a GitHub release.

The legal-info archive is large (it holds the source of every package);
on a private repository it counts towards the account's artifact
storage.

## On the radio

### Start-up

The card holds the system (`x6100-root`, read-only) and a `x6100-data`
partition. On the first start the rest of the card becomes a FAT32
partition labelled **X6100**, mounted at `/mnt`, readable from a PC,
with a commented `x6100.conf` and a `README.txt`.

The screen then shows the launcher:

- **▶ Quake III Arena** (and Team Arena, OpenArena, mods, as far as their
  data is there): play. The line says which files are still missing.
- **Wi-Fi**: scan, connect (password with the knobs or a keyboard),
  forget, on/off.
- **Bluetooth**: scan, pair (keyboards, mice, headphones), connect,
  forget, on/off.
- **Power off**.

The bottom line shows the web page's address once there is a network.

### Controls

| On the radio | Keyboard / mouse | |
|---|---|---|
| VFO, MFK or VOL knob | arrows, mouse wheel | move |
| VOL or MFK push, right key under the screen | Enter, left click | choose |
| left key under the screen | Esc, right click | back |
| the keys under the screen | F1 to F5 | as labelled on the screen |
| power key, short | | stop the game; in the menu: the Power screen, again to switch off |
| power key, held 1.5 s | | switch off |

In the game itself a USB keyboard and mouse (on the USB-A port, through
a powered hub if you use several devices) or Bluetooth ones paired in
the launcher are the controls. Leave the game from its menu or with a
short press of the power key.

Entering a Wi-Fi password with the knobs: VFO turns through the
characters, MFK turns jump between a-z, A-Z, 0-9 and symbols, a VOL or
MFK push adds the character.

### Web page

`http://x6100.local/` (or the address on the screen) from a computer or
phone on the same network:

- **Game data**: choose your game's folder (or single `.pk3` files) and
  they are sent to `quake3/<game>/` on the X6100 partition, with
  progress; files already there are skipped; delete files.
- **Play / Stop**, **Wi-Fi**, **Bluetooth**, **Power off**, as on the
  screen.
- **Licences** of the software in the system.

Anyone on your network can use the page, as anyone can try SSH. Set
`WEB_PASSWORD` in `x6100.conf` to require a password, or `WEB=off` to
switch the page off (the `quake3` command keeps working on the radio).

### Game data

| Where (on the X6100 partition) | What | From |
|---|---|---|
| `quake3/baseq3/pak0.pk3` | the game's data | your Quake III Arena (CD or a store's copy) |
| `quake3/baseq3/pak1.pk3` … `pak8.pk3` | 1.32 point release | [ioquake3.org/extras/patch-data](https://ioquake3.org/extras/patch-data/) (accept its licence, unzip) |
| `quake3/missionpack/pak0.pk3` … `pak3.pk3` | Team Arena, optional | your Team Arena, the point release |
| `quake3/baseoa/*.pk3` | OpenArena, optional | [openarena.ws](https://openarena.ws/) (free, no Quake 3 data needed) |
| `quake3/<mod>/*.pk3` | mods | wherever the mod comes from |

Upload through the web page, or copy them onto the card from a PC.
`/data/quake3/` holds the same layout when you preloaded data into your
own image. The game's settings (`q3config.cfg`), screenshots and demos
are written to the X6100 partition too.

### Quake 3

The Mali-400 MP2 runs ioquake3's OpenGL1 renderer through Mesa's `lima`
driver: desktop GL 2.1 over EGL/GBM and SDL2's KMS/DRM driver, no X11
or Wayland.

- **Picture**: the panel is 480x800, mounted sideways, and the A33's
  display engine cannot rotate it. The game renders 800x480 into a
  framebuffer object and draws that turned (`r_rotate`, a patch to
  ioquake3, `package/ioquake3/`). The same pass applies the gamma ramp,
  since KMS/DRM gives SDL no hardware gamma. `QUAKE3_ROTATE=90` if the
  game comes out upside down; `QUAKE3_ARGS="+set r_rotateGamma 0"`
  turns the gamma pass off.
- **Sound** goes to PipeWire: Bluetooth headphones or speakers paired in
  the launcher, or a USB sound card. Without one the game is silent.
- **From a shell**: `quake3 [GAME]`, `quake3 stop`, `quake3 list`.
- **At power-on**: `QUAKE3=boot` in `x6100.conf` starts `QUAKE3_GAME`
  instead of showing the menu first.
- Logs: `/data/log/quake3.log` (the game), `/data/log/launcher.log`.

### The radio part stays off

This system never transmits and does not run the radio. Linux keeps the
BASE board (the radio's transceiver) in a safe state and otherwise leaves
it alone: the CW key line is held up (released) and the BASE out of reset
(`S03system`); the BASE's data UART is not enabled; the A33 audio codec,
whose output is the transmitter's modem input, is switched off in the
device tree, so no sound can reach it. The one thing sent to the BASE is
the power-off request (`x6100-poweroff`), as the BASE holds the power.

### Partitions

| | | |
|---|---|---|
| p1 `x6100-root` | ext4, read-only `/` | kernel, DTB and `extlinux.conf` in `/boot` |
| p2 `x6100-data` | ext4, `/data` | overlays that make `/etc`, `/var/lib` and `/root` writable (Wi-Fi networks, Bluetooth pairings, SSH host keys); logs; preloaded game data |
| p3 `X6100` | FAT32, `/mnt` | `x6100.conf`, `quake3/` (game data, the game's settings) |

Pulling the power cannot damage the system. `x6100-factory-reset --yes`
forgets the system state (not the game data). `x6100-rw` makes `/`
writable for experiments.

### Access

- SSH as root to `x6100.local` (or the address on the screen): password
  `123`, or a key from `authorized_keys` on the X6100 partition. Set
  `SSH_PASSWORD` in `x6100.conf` to change the password; it is applied
  at boot and then blanked in the file.
- Serial console on the USB-C port (first of its two serial ports) and
  on the test pads (UART0, PB0/PB1), 115200 8N1.
- `x6100-diag` writes a report to the X6100 partition (also a minute
  after every start, `DIAG_AT_BOOT`).

### Wi-Fi and Bluetooth

The internal module is an RTL8723BU (Wi-Fi and Bluetooth in one USB
device), driven by mainline `rtl8xxxu` and `btusb`/`btrtl`. Set `COUNTRY`
in `x6100.conf` to get your country's Wi-Fi channels. From a shell:
`nmcli device wifi connect SSID password PASS`, and `x6100-bt scan`,
`x6100-bt pair MAC`, `x6100-bt devices`, `x6100-bt on|off`. The radio is
never discoverable or pairable on its own: `x6100-btd` accepts pairing
only during the window that `x6100-bt pair` (and the launcher) opens.

## Repository

```
Makefile                      wrapper: Buildroot download, configure, build, flash
configs/x6100_defconfig       the Buildroot configuration
board/x6100/
  dts/allwinner/…x6100.dts    kernel device tree (written for 6.18)
  uboot/                      U-Boot device tree and config fragment
  linux/linux.fragment        kernel options on top of sunxi_defconfig
  patches/linux/              JLT4013A panel for the ST7701 driver;
                              btusb event ordering (LE pairing); sunxi
                              pinctrl keeping IRQ lines interrupting (keypad)
  patches/uboot/              test DRAM after set-up, restart if it fails
  rootfs-overlay/             init scripts, configs, quake3, x6100-bt, helpers
  genimage.cfg, post-*.sh     image layout
package/
  x6100-launcher/             builds launcher/ with Buildroot's Rust
  x6100-btd/                  Bluetooth pairing agent (C, GDBus)
  ioquake3/                   the game engine and the rotation patch
launcher/                     the start screen and web page (Rust)
docker/, scripts/             pinned build container, docker-build.sh
.github/workflows/image.yml   CI: checks, image, releases
LICENSES/, REUSE.toml         licence texts, which file is under which
secrets/                      gitignored, see above
```

## Not done yet

- **Warm reboot does not come back.** `reboot` shuts down cleanly but
  the SoC never restarts. The launcher only offers power off; switch on
  again with the power key.
- **DRAM set-up fails now and then** at power-on (blank screen); press
  the power key again. U-Boot restarts itself when it notices.
- **No Wi-Fi access point**: `rtl8xxxu` has no AP mode for the 8723BU,
  so the radio needs an existing network (or a USB Ethernet adapter).
- **Changing the game's resolution** in its menu is not supported (the
  picture is sized for the panel).

## Licence

This repository is free software. Unless a file says otherwise, it is
licensed under the GNU General Public License, version 2 or (at your
option) any later version, like Buildroot, which it extends. Exceptions:

| Files | Licence |
|---|---|
| `launcher/`, `package/x6100-btd/src/` | MIT |
| `launcher/fonts/10x20.bdf` | public domain (X11 misc-fixed font, see `launcher/fonts/README.md`) |
| `board/x6100/dts/`, `board/x6100/uboot/*.dts` | GPL-2.0-or-later OR MIT |
| `board/x6100/patches/linux/` | GPL-2.0-only, as Linux |
| `board/x6100/patches/uboot/` | GPL-2.0-or-later, as U-Boot |
| `package/ioquake3/*.patch` | GPL-2.0-or-later, as ioquake3 |

The texts are in `LICENSES/`; `REUSE.toml` states the same per path
([REUSE](https://reuse.software/) format). The software the build
downloads (Buildroot, Linux, U-Boot, Mesa, SDL, ioquake3, …) comes under
its own licences; `make legal-info` collects them, see
[Your own card and images for others](#your-own-card-and-images-for-others).

**Trademarks.** Quake and Quake III Arena are trademarks of id Software
LLC, a ZeniMax Media company. Xiegu and X6100 are trademarks of their
owner. This is an independent project, not affiliated with, endorsed by
or sponsored by either; the names are used only to say what the software
runs and on which hardware. ioquake3 is a community project based on the
Quake III Arena source code that id Software released under the GPL. No
game data, logos or other copyrighted game content are part of this
repository or of the images it builds.
