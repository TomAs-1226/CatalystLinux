################################################################################
#
# catalyst-unitv
#
################################################################################

# What the shell installs on the smart camera (an M5Stack UnitV, K210) over
# USB-A, with no microSD card (unitv.rs): in /usr/share/catalyst/unitv,
#  - maixpy.bin: Sipeed's MaixPy v0.6.3 firmware, its build for the
#    M5StickV (the UnitV runs the M5StickV's firmware); MIT and Apache-2.0
#    (MaixPy-v1's LICENSE.md), source at github.com/sipeed/MaixPy-v1, tag
#    v0.6.3. Flashed over the K210's ISP only when the camera does not run a
#    MaixPy with what main.py needs.
#  - face.kmodel: Sipeed's YOLOv2 face detector (MaixPy_scripts, MIT), out
#    of its face_model_at_0x300000.kfpkg, written to 0x300000.
#  - isp.bin: the K210's flash-mode loader, the ISP_PROG kflash.py carries
#    (Kendryte's kflash.py, MIT), loaded into its SRAM to write the flash.
#  - main.py: the camera's program (tools/unitv), written to /flash.
#  - boot.py: what runs before it there (tools/unitv): finishes an update
#    over the Grove port cut off part way, rolls back one that never starts.
# Each file's source and licence are in SOURCES with them.
CATALYST_UNITV_VERSION = 0.6.3
CATALYST_UNITV_MAIXPY = maixpy_v0.6.3_2_gd8901fd22
CATALYST_UNITV_SITE = https://dl.sipeed.com/fileList/MAIX/MaixPy/release/master/$(CATALYST_UNITV_MAIXPY)
CATALYST_UNITV_SOURCE = $(CATALYST_UNITV_MAIXPY)_m5stickv.bin
CATALYST_UNITV_SCRIPTS_COMMIT = 8d7c5f98ba654d226f433d31718d852674cab08d
CATALYST_UNITV_KFLASH_COMMIT = 5783d8d85e7397007a3ba1394a93d3f9f8ad982f
CATALYST_UNITV_EXTRA_DOWNLOADS = \
	https://raw.githubusercontent.com/sipeed/MaixPy_scripts/$(CATALYST_UNITV_SCRIPTS_COMMIT)/machine_vision/models/face_model_at_0x300000.kfpkg \
	https://raw.githubusercontent.com/kendryte/kflash.py/$(CATALYST_UNITV_KFLASH_COMMIT)/kflash.py
CATALYST_UNITV_LICENSE = MIT, Apache-2.0 (MaixPy), MIT (face model, kflash.py ISP loader)
CATALYST_UNITV_LICENSE_FILES = licences/MaixPy-LICENSE.md licences/MaixPy_scripts-LICENSE licences/kflash.py-LICENSE
CATALYST_UNITV_DEPENDENCIES = host-python3
CATALYST_UNITV_MAIN = $(BR2_EXTERNAL_CATALYST_PATH)/../tools/unitv/main.py
CATALYST_UNITV_BOOT = $(BR2_EXTERNAL_CATALYST_PATH)/../tools/unitv/boot.py

define CATALYST_UNITV_EXTRACT_CMDS
	cp $(CATALYST_UNITV_DL_DIR)/$(CATALYST_UNITV_SOURCE) $(@D)/maixpy.bin
	cp $(CATALYST_UNITV_DL_DIR)/face_model_at_0x300000.kfpkg \
		$(CATALYST_UNITV_DL_DIR)/kflash.py $(@D)/
	cp -r $(CATALYST_UNITV_PKGDIR)/licences $(@D)/
endef

define CATALYST_UNITV_BUILD_CMDS
	rm -rf $(@D)/kfpkg
	$(HOST_DIR)/bin/python3 -m zipfile -e $(@D)/face_model_at_0x300000.kfpkg $(@D)/kfpkg
	cp $(@D)/kfpkg/facedetect.kmodel $(@D)/face.kmodel
	$(HOST_DIR)/bin/python3 $(CATALYST_UNITV_PKGDIR)/isp-prog.py $(@D)/kflash.py $(@D)/isp.bin
endef

define CATALYST_UNITV_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0644 $(@D)/maixpy.bin $(TARGET_DIR)/usr/share/catalyst/unitv/maixpy.bin
	$(INSTALL) -D -m 0644 $(@D)/face.kmodel $(TARGET_DIR)/usr/share/catalyst/unitv/face.kmodel
	$(INSTALL) -D -m 0644 $(@D)/isp.bin $(TARGET_DIR)/usr/share/catalyst/unitv/isp.bin
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_MAIN) $(TARGET_DIR)/usr/share/catalyst/unitv/main.py
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_BOOT) $(TARGET_DIR)/usr/share/catalyst/unitv/boot.py
	echo "$(CATALYST_UNITV_MAIXPY)_m5stickv" >$(TARGET_DIR)/usr/share/catalyst/unitv/maixpy.version
	$(INSTALL) -D -m 0644 $(CATALYST_UNITV_PKGDIR)/SOURCES $(TARGET_DIR)/usr/share/catalyst/unitv/SOURCES
	$(INSTALL) -d $(TARGET_DIR)/usr/share/catalyst/unitv/licences
	$(INSTALL) -m 0644 $(@D)/licences/* $(TARGET_DIR)/usr/share/catalyst/unitv/licences/
endef

$(eval $(generic-package))
