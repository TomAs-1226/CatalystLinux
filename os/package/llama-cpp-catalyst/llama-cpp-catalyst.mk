################################################################################
#
# llama.cpp, built for the CardputerZero's processor
#
################################################################################

# Not a package of its own: options added to Buildroot's llama-cpp (this file
# is read after Buildroot's packages, and a package's CMake options are only
# expanded when it is configured).
#
# The CM0's RP3A0 is a Cortex-A53: ARMv8.0-A with CRC and NEON, and none of
# the optional dot product, int8 matrix, fp16 arithmetic or crypto
# instructions. ggml picks its kernels at compile time from the -march it is
# built with, so that is pinned here rather than left to whatever the build
# machine or CMake guesses (and the emulator's A53 has extras, so a wrong
# choice would not show there). One CPU build, no runtime-loaded backends, no
# OpenMP runtime to carry, no model downloads of its own (the shell fetches
# and checks models itself).
#
# The version is Buildroot's (b8117), which runs every model Settings offers
# (SmolLM2's llama, Liquid's LFM2.5 as lfm2, with its tool-call parser) and
# was measured with them on the handheld. How llama-server is run (threads,
# priority, batches, cache) is in userland/crates/localmodel.
ifeq ($(BR2_PACKAGE_LLAMA_CPP),y)
LLAMA_CPP_CONF_OPTS += \
	-DGGML_NATIVE=OFF \
	-DGGML_CPU_ARM_ARCH=armv8-a+crc \
	-DGGML_CPU_KLEIDIAI=OFF \
	-DGGML_CPU_ALL_VARIANTS=OFF \
	-DGGML_BACKEND_DL=OFF \
	-DGGML_OPENMP=OFF \
	-DLLAMA_CURL=OFF
endif
