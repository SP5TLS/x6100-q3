# x6100-q3: build the X6100 SD card image (Linux + Quake 3).
#
#   make                 configure (first time) and build output/images/sdcard.img
#   make menuconfig      Buildroot configuration (then: make savedefconfig)
#   make linux-menuconfig / x6100-launcher-rebuild / <pkg>-rebuild ...
#                        passed to Buildroot
#   make legal-info      licences, sources and manifest of everything in the
#                        image (output/legal-info), for sharing an image
#   make flash DEV=/dev/sdX   write the image to an SD card (Linux host)
#
# On macOS (or anywhere without Buildroot's host tools) use
# scripts/docker-build.sh, which runs this Makefile in a container.

BR_VERSION := 2026.02.3
BR_SHA256  := 5a59e7501b0b4ec52c41f4bfa79412320e0b37eae5f719605a258e8d0c6fc7fb
BR_TARBALL := buildroot-$(BR_VERSION).tar.xz
BR_URL     := https://buildroot.org/downloads/$(BR_TARBALL)

TOP     := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
BR_DIR  ?= $(TOP)/buildroot
O       ?= $(TOP)/output
DL      ?= $(TOP)/dl
DEFCONFIG ?= x6100_defconfig

export BR2_DL_DIR := $(DL)
BR_MAKE = $(MAKE) -C $(BR_DIR) O=$(O) BR2_EXTERNAL=$(TOP)

.PHONY: all flash clean distclean help
all: $(O)/.config
	$(BR_MAKE)
	@echo
	@echo "Image: $(O)/images/sdcard.img"

# The stamp names the version, so a tree of another version is replaced.
$(BR_DIR)/.x6100-q3-$(BR_VERSION):
	mkdir -p $(DL) $(BR_DIR)
	test -f $(DL)/$(BR_TARBALL) || wget -nv -O $(DL)/$(BR_TARBALL) $(BR_URL)
	echo "$(BR_SHA256)  $(DL)/$(BR_TARBALL)" | sha256sum -c -
	find $(BR_DIR) -mindepth 1 -delete
	tar -xf $(DL)/$(BR_TARBALL) -C $(BR_DIR) --strip-components=1
	touch $@

$(O)/.config: $(BR_DIR)/.x6100-q3-$(BR_VERSION) $(TOP)/configs/$(DEFCONFIG)
	$(BR_MAKE) $(DEFCONFIG)

flash:
	@test -n "$(DEV)" || { echo "usage: make flash DEV=/dev/sdX"; exit 1; }
	@test -b "$(DEV)" || { echo "$(DEV) is not a block device"; exit 1; }
	@echo "Writing $(O)/images/sdcard.img to $(DEV) (all data on it is lost)"
	@read -p "Type yes to continue: " a && [ "$$a" = yes ]
	dd if=$(O)/images/sdcard.img of=$(DEV) bs=4M conv=fsync status=progress

# The output directory may be a mount point (Docker volume): empty it.
clean:
	mkdir -p $(O)
	find $(O) -mindepth 1 -delete

distclean: clean
	rm -rf $(BR_DIR)

help:
	@sed -n '2,13s/^# \{0,1\}//p' $(TOP)/Makefile

# Anything else is a Buildroot target.
%: $(O)/.config
	$(BR_MAKE) $@
