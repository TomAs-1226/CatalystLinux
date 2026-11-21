################################################################################
#
# m5stack-cardputer-zero
#
################################################################################

M5STACK_CARDPUTER_ZERO_VERSION = 4f7aeee9fc103ebb03d22cac476bb191ecfb321e
M5STACK_CARDPUTER_ZERO_SITE = $(call github,m5stack,m5stack-linux-dtoverlays,$(M5STACK_CARDPUTER_ZERO_VERSION))
M5STACK_CARDPUTER_ZERO_LICENSE = GPL-2.0
M5STACK_CARDPUTER_ZERO_LICENSE_FILES = COPYING
M5STACK_CARDPUTER_ZERO_INSTALL_IMAGES = YES

M5STACK_CARDPUTER_ZERO_MODULE_SUBDIRS = \
	modules/m5ioe1-1.0 \
	modules/st7789v-1.0 \
	modules/tca8418-1.0 \
	modules/bq27220-1.0 \
	modules/CardputerZero
M5STACK_CARDPUTER_ZERO_MODULE_MAKE_OPTS = CONFIG_CARDPUTERO_V0_5=y

# Build only what Catalyst Linux uses: the DRM panel driver rather than the
# old fbtft one, and no audio machine driver until audio is brought up.
define M5STACK_CARDPUTER_ZERO_TRIM_MODULES
	$(SED) 's/^kermel_ko *:=.*/kermel_ko := pwm_bl_m5stack.ko panel-mipi-dbi-m.ko/' \
		$(@D)/modules/st7789v-1.0/Makefile
	$(SED) 's/^kermel_ko *:=.*/kermel_ko := powerfail_suo.ko gpio-forwarder.ko/' \
		$(@D)/modules/CardputerZero/Makefile
endef
M5STACK_CARDPUTER_ZERO_POST_PATCH_HOOKS += M5STACK_CARDPUTER_ZERO_TRIM_MODULES

# The board's own overlays. (The stock ones config.txt names come from the
# kernel build, which installs the whole overlays directory.)
# $(1): source .dts, $(2): overlay name, $(3): extra preprocessor flags
define M5STACK_CARDPUTER_ZERO_DTBO
	$(HOSTCC) -E -P -nostdinc -undef -x assembler-with-cpp $(3) \
		-o $(@D)/dtbo/$(2).pp $(1)
	$(LINUX_DIR)/scripts/dtc/dtc -@ -q -I dts -O dtb -o $(@D)/dtbo/$(2).dtbo $(@D)/dtbo/$(2).pp
endef

define M5STACK_CARDPUTER_ZERO_BUILD_OVERLAYS
	mkdir -p $(@D)/dtbo
	$(call M5STACK_CARDPUTER_ZERO_DTBO,$(@D)/modules/CardputerZero/cardputerzero-v5-overlay.dts,cardputerzero-v5-overlay,-DCONFIG_CARDPUTERO_V0_5=1)
	$(call M5STACK_CARDPUTER_ZERO_DTBO,$(@D)/modules/bq27220-1.0/bq27220_v5.dts,bq27220_v5,)
	# Catalyst Linux's own: the panel driven tear-free (Developer options >
	# Tear-free panel adds it after cardputerzero-v5-overlay).
	$(call M5STACK_CARDPUTER_ZERO_DTBO,$(M5STACK_CARDPUTER_ZERO_PKGDIR)/cardputerzero-portrait-overlay.dts,cardputerzero-portrait,)
endef
M5STACK_CARDPUTER_ZERO_POST_BUILD_HOOKS += M5STACK_CARDPUTER_ZERO_BUILD_OVERLAYS

define M5STACK_CARDPUTER_ZERO_INSTALL_IMAGES_CMDS
	mkdir -p $(BINARIES_DIR)/rpi-firmware/overlays
	$(INSTALL) -m 0644 $(@D)/dtbo/*.dtbo $(BINARIES_DIR)/rpi-firmware/overlays/
endef

# The panel driver loads its init sequence as firmware, named after the
# panel's compatible: M5Stack's, and the tear-free mode's (MADCTL 0x00; made
# from M5Stack's by scripts/panel-portrait-blob.py).
define M5STACK_CARDPUTER_ZERO_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0644 "$(@D)/modules/st7789v-1.0/cardputerzero,st7789v_lcd.bin" \
		"$(TARGET_DIR)/lib/firmware/cardputerzero,st7789v_lcd.bin"
	$(INSTALL) -D -m 0644 "$(M5STACK_CARDPUTER_ZERO_PKGDIR)/cardputerzero,st7789v_portrait.bin" \
		"$(TARGET_DIR)/lib/firmware/cardputerzero,st7789v_portrait.bin"
endef

$(eval $(kernel-module))
$(eval $(generic-package))
