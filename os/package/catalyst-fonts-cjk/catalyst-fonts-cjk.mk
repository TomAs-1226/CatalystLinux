################################################################################
#
# catalyst-fonts-cjk
#
################################################################################

# Noto Sans CJK (JP), variable, cut down to the Chinese, Japanese and Korean
# characters in characters.txt (13,483 of them; 5.5 MB from 30.8 MB), for
# the shell to draw what its own Latin fonts lack. The shell maps the file
# and reads it only once a character from it is drawn.
#
# Cut here with fontTools (its pure-Python wheel, pinned) on Buildroot's
# host Python. scripts/cjk-font.py makes the same file for the simulator.
CATALYST_FONTS_CJK_VERSION = 2.004
CATALYST_FONTS_CJK_COMMIT = 523d033d6cb47f4a80c58a35753646f5c3608a78
CATALYST_FONTS_CJK_SITE = https://github.com/notofonts/noto-cjk/raw/$(CATALYST_FONTS_CJK_COMMIT)/Sans/Variable/OTF
CATALYST_FONTS_CJK_SOURCE = NotoSansCJKjp-VF.otf
CATALYST_FONTS_CJK_FONTTOOLS = fonttools-4.60.0-py3-none-any.whl
CATALYST_FONTS_CJK_EXTRA_DOWNLOADS = \
	https://github.com/notofonts/noto-cjk/raw/$(CATALYST_FONTS_CJK_COMMIT)/LICENSE \
	https://files.pythonhosted.org/packages/f9/a4/247d3e54eb5ed59e94e09866cfc4f9567e274fbf310ba390711851f63b3b/$(CATALYST_FONTS_CJK_FONTTOOLS)
CATALYST_FONTS_CJK_LICENSE = OFL-1.1
CATALYST_FONTS_CJK_LICENSE_FILES = LICENSE
CATALYST_FONTS_CJK_DEPENDENCIES = host-python3

define CATALYST_FONTS_CJK_EXTRACT_CMDS
	cp $(CATALYST_FONTS_CJK_DL_DIR)/$(CATALYST_FONTS_CJK_SOURCE) \
		$(CATALYST_FONTS_CJK_DL_DIR)/LICENSE \
		$(CATALYST_FONTS_CJK_DL_DIR)/$(CATALYST_FONTS_CJK_FONTTOOLS) $(@D)/
endef

define CATALYST_FONTS_CJK_BUILD_CMDS
	rm -rf $(@D)/fonttools
	$(HOST_DIR)/bin/python3 -m zipfile -e $(@D)/$(CATALYST_FONTS_CJK_FONTTOOLS) $(@D)/fonttools
	PYTHONPATH=$(@D)/fonttools $(HOST_DIR)/bin/python3 $(CATALYST_FONTS_CJK_PKGDIR)/subset.py \
		$(@D)/$(CATALYST_FONTS_CJK_SOURCE) $(CATALYST_FONTS_CJK_PKGDIR)/characters.txt \
		$(@D)/NotoSansCJK-Catalyst.otf
endef

define CATALYST_FONTS_CJK_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0644 $(@D)/NotoSansCJK-Catalyst.otf $(TARGET_DIR)/usr/share/fonts/catalyst/NotoSansCJK-Catalyst.otf
	$(INSTALL) -D -m 0644 $(@D)/LICENSE $(TARGET_DIR)/usr/share/fonts/catalyst/OFL-NotoSansCJK.txt
endef

$(eval $(generic-package))
