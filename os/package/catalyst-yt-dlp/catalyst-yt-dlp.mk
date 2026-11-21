################################################################################
#
# catalyst-yt-dlp
#
################################################################################

# yt-dlp's own single-file release (a Python zipapp, with yt-dlp-ejs, the
# script that answers YouTube's player challenges, inside it), for the Music
# app to find a song's audio stream. YouTube changes often, so the Music app
# can bring a newer copy to /data/music (Music > Account > Stream finder);
# this one is the start.
CATALYST_YT_DLP_VERSION = 2026.10.26
CATALYST_YT_DLP_SOURCE = yt-dlp
CATALYST_YT_DLP_SITE = https://github.com/yt-dlp/yt-dlp/releases/download/$(CATALYST_YT_DLP_VERSION)
CATALYST_YT_DLP_LICENSE = Unlicense, MIT (astring), ISC (meriyah)

define CATALYST_YT_DLP_EXTRACT_CMDS
	cp $(CATALYST_YT_DLP_DL_DIR)/$(CATALYST_YT_DLP_SOURCE) $(@D)/yt-dlp
endef

define CATALYST_YT_DLP_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/yt-dlp $(TARGET_DIR)/usr/libexec/catalyst/yt-dlp
endef

$(eval $(generic-package))
