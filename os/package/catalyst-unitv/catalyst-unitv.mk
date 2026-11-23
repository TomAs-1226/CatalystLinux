################################################################################
#
# catalyst-unitv
#
################################################################################

# What the shell installs on the smart camera (an M5Stack UnitV, K210) over
# USB-A, and sends over the Grove port later (unitv.rs, smartcam.rs): in
# /usr/share/catalyst/unitv,
#  - catalyst-coprocessor.bin: Catalyst Coprocessor, Catalyst's own MaixPy
#    firmware (tools/unitv/firmware: MaixPy at d8901fd22, MIT and
#    Apache-2.0, trimmed to the UnitV, with the `catalyst` module and a
#    frozen boot). Built by tools/unitv/firmware/build.sh from pinned and
#    hashed sources (the K210's toolchain is not one of Buildroot's), kept
#    in the source tree with its SHA-256 in catalyst-coprocessor.txt, and
#    checked against it here. Flashed over the K210's ISP when the camera
#    does not run it at firmware.version's number.
#  - face.kmodel: Sipeed's YOLOv2 face detector (MaixPy_scripts, MIT), out
#    of its face_model_at_0x300000.kfpkg, written to 0x300000.
#  - kws.kmodel, kws.json: the keyword spotter for voice commands
#    (tools/unitv/speech: trained on Google's Speech Commands, CC-BY-4.0,
#    compiled by nncase v0.2.0-beta4, Apache-2.0), sent over the Grove port.
#  - isp.bin: the K210's flash-mode loader, the ISP_PROG kflash.py carries
#    (Kendryte's kflash.py, MIT), loaded into its SRAM to write the flash.
#  - main.py: the camera's program (tools/unitv), written to /flash.
# Each file's source and licence are in SOURCES with them.
CATALYST_UNITV_VERSION = 2.0
CATALYST_UNITV_SCRIPTS_COMMIT = 8d7c5f98ba654d226f433d31718d852674cab08d
CATALYST_UNITV_KFLASH_COMMIT = 5783d8d85e7397007a3ba1394a93d3f9f8ad982f
CATALYST_UNITV_SITE = https://raw.githubusercontent.com/sipeed/MaixPy_scripts/$(CATALYST_UNITV_SCRIPTS_COMMIT)/machine_vision/models
CATALYST_UNITV_SOURCE = face_model_at_0x300000.kfpkg
CATALYST_UNITV_EXTRA_DOWNLOADS = \
	https://raw.githubusercontent.com/kendryte/kflash.py/$(CATALYST_UNITV_KFLASH_COMMIT)/kflash.py
CATALYST_UNITV_LICENSE = MIT, Apache-2.0 (Catalyst Coprocessor's MaixPy), MIT (face model, kflash.py ISP loader), CC-BY-4.0 (Speech Commands, the keyword spotter's data)
CATALYST_UNITV_LICENSE_FILES = licences/MaixPy-LICENSE.md licences/MaixPy_scripts-LICENSE licences/kflash.py-LICENSE licences/speech-commands-ATTRIBUTION
CATALYST_UNITV_DEPENDENCIES = host-python3
CATALYST_UNITV_TOOLS = $(BR2_EXTERNAL_CATALYST_PATH)/../tools/unitv
CATALYST_UNITV_FIRMWARE = $(CATALYST_UNITV_TOOLS)/firmware/catalyst-coprocessor.bin
CATALYST_UNITV_FIRMWARE_SHA256 = $(shell sed -n 's/^sha256  //p' $(CATALYST_UNITV_TOOLS)/firmware/catalyst-coprocessor.txt)

define CATALYST_UNITV_EXTRACT_CMDS
	cp $(CATALYST_UNITV_DL_DIR)/face_model_at_0x300000.kfpkg \
		$(CATALYST_UNITV_DL_DIR)/kflash.py $(@D)/
	cp -r $(CATALYST_UNITV_PKGDIR)/licences $(@D)/
endef

define CATALYST_UNITV_BUILD_CMDS
	rm -rf $(@D)/kfpkg
	$(HOST_DIR)/bin/python3 -m zipfile -e $(@D)/face_model_at_0x300000.kfpkg $(@D)/kfpkg
	cp $(@D)/kfpkg/facedetect.kmodel $(@D)/face.kmodel
	$(HOST_DIR)/bin/python3 $(CATALYST_UNITV_PKGDIR)/isp-prog.py $(@D)/kflash.py $(@D)/isp.bin
	echo "$(CATALYST_UNITV_FIRMWARE_SHA256)  $(CATALYST_UNITV_FIRMWARE)" | sha256sum -c --quiet
endef

define CATALYST_UNITV_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_FIRMWARE) $(TARGET_DIR)/usr/share/catalyst/unitv/catalyst-coprocessor.bin
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_TOOLS)/firmware/FIRMWARE $(TARGET_DIR)/usr/share/catalyst/unitv/firmware.version
	$(INSTALL) -D -m 0644 $(@D)/face.kmodel $(TARGET_DIR)/usr/share/catalyst/unitv/face.kmodel
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_TOOLS)/speech/kws.kmodel $(TARGET_DIR)/usr/share/catalyst/unitv/kws.kmodel
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_TOOLS)/speech/kws.json $(TARGET_DIR)/usr/share/catalyst/unitv/kws.json
	$(INSTALL) -D -m 0644 $(@D)/isp.bin $(TARGET_DIR)/usr/share/catalyst/unitv/isp.bin
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_TOOLS)/main.py $(TARGET_DIR)/usr/share/catalyst/unitv/main.py
	rm -f $(TARGET_DIR)/usr/share/catalyst/unitv/maixpy.bin $(TARGET_DIR)/usr/share/catalyst/unitv/maixpy.version $(TARGET_DIR)/usr/share/catalyst/unitv/boot.py
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_PKGDIR)/SOURCES $(TARGET_DIR)/usr/share/catalyst/unitv/SOURCES
	$(INSTALL) -d $(TARGET_DIR)/usr/share/catalyst/unitv/licences
	$(INSTALL) -m 0644 $(@D)/licences/* $(TARGET_DIR)/usr/share/catalyst/unitv/licences/
endef

$(eval $(generic-package))
