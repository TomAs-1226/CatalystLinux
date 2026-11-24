################################################################################
#
# catalyst-camera
#
################################################################################

# Raised with each change to the program: a new version is a new build
# directory, so the next image build compiles it afresh by itself.
CATALYST_CAMERA_VERSION = 1.9
CATALYST_CAMERA_SITE = $(BR2_EXTERNAL_CATALYST_PATH)/package/catalyst-camera/src
CATALYST_CAMERA_SITE_METHOD = local
CATALYST_CAMERA_LICENSE = MIT
# Photos are written as JPEG through libjpeg (libjpeg-turbo, with NEON), on
# three cores (src/photo.h; test/photo-test.cpp checks it under qemu). What a
# photo decides while it is taken is src/still.h (test/still-test.cpp); Night's
# and HDR's merge, src/stack.h (test/stack-test.cpp).
CATALYST_CAMERA_DEPENDENCIES = libcamera jpeg host-pkgconf

# libcamera's flags are asked for when it is built, not when this file is read.
define CATALYST_CAMERA_BUILD_CMDS
	$(TARGET_MAKE_ENV) $(TARGET_CXX) $(TARGET_CXXFLAGS) $(TARGET_LDFLAGS) -std=c++17 -Wall -Wextra -O2 \
		`$(PKG_CONFIG_HOST_BINARY) --cflags libcamera` \
		-o $(@D)/catalyst-camera $(@D)/catalyst-camera.cpp \
		`$(PKG_CONFIG_HOST_BINARY) --libs libcamera` -ljpeg -lpthread
endef

define CATALYST_CAMERA_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/catalyst-camera $(TARGET_DIR)/usr/bin/catalyst-camera
	$(INSTALL) -D -m 0644 $(@D)/vc4.yaml $(TARGET_DIR)/usr/share/catalyst-camera/vc4.yaml
endef

$(eval $(generic-package))
