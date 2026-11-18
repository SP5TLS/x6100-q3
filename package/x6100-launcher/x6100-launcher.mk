################################################################################
#
# x6100-launcher
#
################################################################################

# Built from launcher/ in this tree. It has no crate dependencies, so
# cargo needs nothing beyond Buildroot's toolchain (no vendoring).
X6100_LAUNCHER_VERSION = 1.0.0
X6100_LAUNCHER_SITE = $(BR2_EXTERNAL_X6100_PATH)/launcher
X6100_LAUNCHER_SITE_METHOD = local
X6100_LAUNCHER_LICENSE = MIT, Public Domain (font)
X6100_LAUNCHER_LICENSE_FILES = LICENSE fonts/README.md
# A cargo build on the host leaves target/ next to the sources.
X6100_LAUNCHER_OVERRIDE_SRCDIR_RSYNC_EXCLUSIONS = --exclude /target

$(eval $(cargo-package))
