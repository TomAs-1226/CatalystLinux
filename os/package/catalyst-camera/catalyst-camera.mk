################################################################################
#
# catalyst-camera
#
################################################################################

# Raised with each change to the program: a new version is a new build
# directory, so the next image build compiles it afresh by itself.
CATALYST_CAMERA_VERSION = 1.1
CATALYST_CAMERA_SITE = $(BR2_EXTERNAL_CATALYST_PATH)/package/catalyst-camera/src
CATALYST_CAMERA_SITE_METHOD = local
CATALYST_CAMERA_LICENSE = MIT
CATALYST_CAMERA_DEPENDENCIES = libcamera host-pkgconf

# libcamera's flags are asked for when it is built, not when this file is read.
define CATALYST_CAMERA_BUILD_CMDS
	$(TARGET_MAKE_ENV) $(TARGET_CXX) $(TARGET_CXXFLAGS) $(TARGET_LDFLAGS) -std=c++17 -Wall -Wextra -O2 \
		`$(PKG_CONFIG_HOST_BINARY) --cflags libcamera` \
		-o $(@D)/catalyst-camera $(@D)/catalyst-camera.cpp \
		`$(PKG_CONFIG_HOST_BINARY) --libs libcamera` -lpthread
endef

define CATALYST_CAMERA_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/catalyst-camera $(TARGET_DIR)/usr/bin/catalyst-camera
endef

$(eval $(generic-package))
