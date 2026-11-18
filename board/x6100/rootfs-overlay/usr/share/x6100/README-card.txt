X6100 partition of the SD card (x6100-q3)
=========================================

This partition is /mnt on the radio. Everything here can be read and
changed from a PC.

  x6100.conf          system settings (Wi-Fi country, Bluetooth, time
                      zone, SSH and web passwords, Quake 3 options);
                      read at boot
  authorized_keys     SSH public keys for root (or ssh/authorized_keys)
  quake3/             Quake 3 game data and the game's settings and
                      screenshots, one directory per game:
                        baseq3/pak0.pk3            from your copy of
                                                   Quake III Arena
                        baseq3/pak1.pk3..pak8.pk3  the 1.32 point release
                                                   (ioquake3.org/extras/
                                                   patch-data/)
                        missionpack/pak0..pak3.pk3 Team Arena, optional
                      The launcher's web page (http://x6100.local/) puts
                      uploads here; copying them from a PC works too.
  x6100-diag.txt      diagnostics, written a minute after each boot

System state (Wi-Fi networks, Bluetooth pairings, SSH host keys) lives on
the separate x6100-data partition, not here.
