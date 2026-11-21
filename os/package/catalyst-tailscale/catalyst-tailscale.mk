################################################################################
#
# catalyst-tailscale
#
################################################################################

# The newest release Buildroot's Go (1.27.2) builds: 1.104 wants Go 1.27.4.
CATALYST_TAILSCALE_VERSION = 1.102.3
CATALYST_TAILSCALE_SITE = $(call github,tailscale,tailscale,v$(CATALYST_TAILSCALE_VERSION))
CATALYST_TAILSCALE_LICENSE = BSD-3-Clause
CATALYST_TAILSCALE_LICENSE_FILES = LICENSE
CATALYST_TAILSCALE_GOMOD = tailscale.com

# Buildroot vendors Go modules straight from their repositories, but one of
# Tailscale's (github.com/kenshaw/evdev) is no longer on GitHub: Go's module
# mirror still has it, and go.sum checks every module either way. Only the
# download reads this; the build is offline, from vendor/.
CATALYST_TAILSCALE_GO_ENV = GOPROXY=https://proxy.golang.org,direct

# Only what a client on a kernel TUN needs: the OS router, the CLI inside
# tailscaled, the notification bus `tailscale up` watches for its login URL,
# health, port mapping and raw-socket NAT traversal (getting through home
# routers), and baked-in roots as a fallback. Tailscale's own featuretags
# tool turns that into the ts_omit_* tags for everything else, as its
# build_dist.sh --extra-small does. Static (no cgo), stripped.
CATALYST_TAILSCALE_FEATURES = osrouter,cli,ipnbus,health,portmapper,listenrawdisco,bakedroots

define CATALYST_TAILSCALE_BUILD_CMDS
	cd $(@D); \
	tags=$$($(HOST_GO_HOST_ENV) $(GO_BIN) run ./cmd/featuretags --min --add=$(CATALYST_TAILSCALE_FEATURES)) && \
	echo "catalyst-tailscale tags: $$tags" && \
	$(HOST_GO_TARGET_ENV) CGO_ENABLED=0 $(GO_BIN) build -v -modcacherw -trimpath -buildvcs=false -p $(PARALLEL_JOBS) \
		-tags "$$tags" \
		-ldflags "-s -w -X tailscale.com/version.longStamp=$(CATALYST_TAILSCALE_VERSION) -X tailscale.com/version.shortStamp=$(CATALYST_TAILSCALE_VERSION)" \
		-o $(@D)/bin/tailscaled tailscale.com/cmd/tailscaled
endef

# One binary: run as `tailscale` it is the CLI.
define CATALYST_TAILSCALE_INSTALL_TARGET_CMDS
	$(INSTALL) -D -m 0755 $(@D)/bin/tailscaled $(TARGET_DIR)/usr/sbin/tailscaled
	ln -sf ../sbin/tailscaled $(TARGET_DIR)/usr/bin/tailscale
endef

$(eval $(golang-package))
