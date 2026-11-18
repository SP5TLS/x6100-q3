################################################################################
#
# ioquake3
#
################################################################################

IOQUAKE3_VERSION = 52088b9eae247d8e321c8dc1fb49a879652854f9
IOQUAKE3_SITE = $(call github,ioquake,ioq3,$(IOQUAKE3_VERSION))
IOQUAKE3_LICENSE = GPL-2.0+
IOQUAKE3_LICENSE_FILES = COPYING.txt
IOQUAKE3_DEPENDENCIES = sdl2 zlib jpeg libogg libvorbis opus opusfile host-pkgconf

# Client and OpenGL1 renderer only. The game modules come as QVMs in the
# game data (run by the ARMv7 JIT); no dedicated server, no OpenAL (SDL
# audio), no HTTP downloads, VoIP or Mumble. Libraries from Buildroot
# (USE_INTERNAL_LIBS=OFF), so legal-info lists each with its licence.
IOQUAKE3_CONF_OPTS = \
	-DBUILD_SERVER=OFF \
	-DBUILD_CLIENT=ON \
	-DBUILD_RENDERER_GL1=ON \
	-DBUILD_RENDERER_GL2=OFF \
	-DBUILD_GAME_LIBRARIES=OFF \
	-DBUILD_GAME_QVMS=OFF \
	-DUSE_RENDERER_DLOPEN=ON \
	-DUSE_OPENAL=OFF \
	-DUSE_HTTP=OFF \
	-DUSE_VOIP=OFF \
	-DUSE_MUMBLE=OFF \
	-DUSE_CODEC_VORBIS=ON \
	-DUSE_CODEC_OPUS=ON \
	-DUSE_INTERNAL_LIBS=OFF \
	-DPRODUCT_VERSION=1.36_g52088b9_x6100

# The renderer is loaded from the binary's directory.
define IOQUAKE3_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(IOQUAKE3_BUILDDIR)/Release/ioquake3 \
		$(TARGET_DIR)/usr/lib/ioquake3/ioquake3
	$(INSTALL) -D -m 0755 $(IOQUAKE3_BUILDDIR)/Release/renderer_opengl1.so \
		$(TARGET_DIR)/usr/lib/ioquake3/renderer_opengl1.so
endef

$(eval $(cmake-package))
