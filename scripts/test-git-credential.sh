#!/bin/sh
# Checks the handheld's git credential helper on a computer: it gives the
# GitHub key, with its login, to https://github.com only, ignores store and
# erase, and says nothing at all for any other server or with no key.
#
#   sh scripts/test-git-credential.sh
set -eu
HERE="$(cd "$(dirname "$0")/.." && pwd)"
HELPER="$HERE/os/board/cardputer-zero/rootfs-overlay/usr/libexec/catalyst/git-credential"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK:?}"' EXIT
export CATALYST_KEYS="$WORK/keys" CATALYST_GITHUB="$WORK/github"
printf 'tba=tba-key\ngithub=ghp_TESTTOKEN123\nopenai=sk-x\n' >"$CATALYST_KEYS"
printf 'login=team5805\nname=Team 5805\nemail=1+team5805@users.noreply.github.com\n' >"$CATALYST_GITHUB"

fail() {
	echo "FAIL: $*" >&2
	exit 1
}
ask() { printf '%s' "$2" | sh "$HELPER" "$1"; }

out=$(ask get 'protocol=https
host=github.com
path=team5805/robot.git

')
[ "$out" = "username=team5805
password=ghp_TESTTOKEN123" ] || fail "github.com got: $out"

for other in 'protocol=https
host=gitlab.com' 'protocol=http
host=github.com' 'protocol=https
host=github.com.evil.example' 'protocol=https
host=evil.example'; do
	out=$(ask get "$other

")
	[ -z "$out" ] || fail "another server got an answer: $other -> $out"
done

for verb in store erase; do
	out=$(ask "$verb" 'protocol=https
host=github.com
username=x
password=y

')
	[ -z "$out" ] || fail "$verb answered: $out"
done

# No login kept yet: GitHub takes any name with a token.
rm -f "$CATALYST_GITHUB"
out=$(ask get 'protocol=https
host=github.com

')
[ "$out" = "username=x-access-token
password=ghp_TESTTOKEN123" ] || fail "without a login: $out"

# No key: nothing.
printf 'tba=tba-key\n' >"$CATALYST_KEYS"
out=$(ask get 'protocol=https
host=github.com

')
[ -z "$out" ] || fail "answered with no key: $out"
echo "git-credential: all checks pass"
