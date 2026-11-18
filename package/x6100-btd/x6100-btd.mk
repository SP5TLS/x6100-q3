################################################################################
#
# x6100-btd
#
################################################################################

X6100_BTD_VERSION = 1.0
X6100_BTD_SITE = $(BR2_EXTERNAL_X6100_PATH)/package/x6100-btd/src
X6100_BTD_SITE_METHOD = local
X6100_BTD_LICENSE = MIT
X6100_BTD_LICENSE_FILES = LICENSE
X6100_BTD_DEPENDENCIES = libglib2 host-pkgconf

define X6100_BTD_BUILD_CMDS
	$(TARGET_CC) $(TARGET_CFLAGS) $(TARGET_LDFLAGS) -Wall -Wextra \
		$$($(PKG_CONFIG_HOST_BINARY) --cflags gio-unix-2.0) \
		-o $(@D)/x6100-btd $(@D)/x6100-btd.c \
		$$($(PKG_CONFIG_HOST_BINARY) --libs gio-unix-2.0)
endef

define X6100_BTD_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/x6100-btd $(TARGET_DIR)/usr/sbin/x6100-btd
endef

$(eval $(generic-package))
