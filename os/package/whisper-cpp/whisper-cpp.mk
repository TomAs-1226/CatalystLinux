################################################################################
#
# whisper-cpp
#
################################################################################

# whisper.cpp's whisper-server, for the shell's offline dictation
# (userland/apps/shell/src/whisper_local.rs): Whisper tiny.en, downloaded to
# /data/models when asked for, and running only while dictation needs it.
#
# Built the way llama-cpp-catalyst builds llama.cpp, for the CM0's
# Cortex-A53 (ARMv8.0-A with CRC and NEON; no dot product, int8 matrix,
# fp16 arithmetic or crypto): ggml's kernels pinned to -march=armv8-a+crc,
# one CPU build, no runtime-loaded backends, no OpenMP, no curl, SDL or
# ffmpeg. Static (BUILD_SHARED_LIBS=OFF): whisper.cpp brings a ggml of its
# own (0.26.0), and llama-cpp's is in /usr/lib as libggml*.so (0.9.7);
# linked into the one program, whisper's never meets llama's, and nothing
# of it but /usr/bin/whisper-server is installed.
#
# ggml_backend_load_all(), the first thing whisper-server does, looks for
# libggml-*.so beside the program and in the current directory (not in
# /usr/lib): the shell starts it in /, so it never finds llama's.
#
# The patches: a per-request max_tokens, to bound each decode by the length
# of the speech; and flash attention over exactly the audio encoded
# (upstream reads up to 255 positions past it, left over from earlier
# requests, for the sake of GPU kernels this build does not have).
WHISPER_CPP_VERSION = 1.9.5
WHISPER_CPP_SITE = $(call github,ggml-org,whisper.cpp,v$(WHISPER_CPP_VERSION))
WHISPER_CPP_LICENSE = MIT
WHISPER_CPP_LICENSE_FILES = LICENSE
WHISPER_CPP_CPE_ID_VENDOR = ggml
WHISPER_CPP_CPE_ID_PRODUCT = whisper.cpp
WHISPER_CPP_SUPPORTS_IN_SOURCE_BUILD = NO

WHISPER_CPP_CONF_OPTS = \
	-DBUILD_SHARED_LIBS=OFF \
	-DWHISPER_BUILD_IS_DEV=OFF \
	-DWHISPER_BUILD_TESTS=OFF \
	-DWHISPER_BUILD_EXAMPLES=ON \
	-DWHISPER_CURL=OFF \
	-DWHISPER_SDL2=OFF \
	-DWHISPER_COMMON_FFMPEG=OFF \
	-DWHISPER_FATAL_WARNINGS=OFF \
	-DGGML_NATIVE=OFF \
	-DGGML_CPU_ARM_ARCH=armv8-a+crc \
	-DGGML_CPU_KLEIDIAI=OFF \
	-DGGML_CPU_ALL_VARIANTS=OFF \
	-DGGML_BACKEND_DL=OFF \
	-DGGML_OPENMP=OFF \
	-DGGML_BLAS=OFF \
	-DGGML_VULKAN=OFF \
	-DGGML_CCACHE=OFF

# The examples are one switch in whisper.cpp's CMake: of them, only the
# server is compiled.
WHISPER_CPP_BUILD_OPTS = --target whisper-server

define WHISPER_CPP_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(WHISPER_CPP_BUILDDIR)/bin/whisper-server $(TARGET_DIR)/usr/bin/whisper-server
endef

$(eval $(cmake-package))
