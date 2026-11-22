#!/bin/sh
# catalyst-audio against a stand-in sound card: amixer, aplay, modprobe and
# bluealsa-cli are small scripts that keep what they were asked in a scratch
# folder, and /proc/asound and /data are folders there too. Checks the
# volume (saved, clamped, muted and back), the microphone's choices, ALSA's
# configuration for the speaker and for Bluetooth headphones (only while
# connected), the status lines, and start (drivers loaded, volume control
# made before any sound, both restored). Run anywhere with a POSIX shell,
# sed and awk (WSL is fine): sh scripts/test-catalyst-audio.sh
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/data" "$T/run" "$T/proc" "$T/share/sounds"
cp "$HERE/os/board/cardputer-zero/rootfs-overlay/usr/share/catalyst/asound.conf" "$T/share/"
: >"$T/share/sounds/chime.wav"

# amixer: cget answers from files ("master" once the control exists, "jack"
# for the headphones); cset keeps what it was given.
cat >"$T/bin/amixer" <<EOF
#!/bin/sh
echo "\$*" >>"$T/amixer.log"
for a in "\$@"; do
	case "\$a" in
	"name=Master Playback Volume") control=master ;;
	"name=Headphones Jack") control=jack ;;
	"name=PGAL Select") control=pgal ;;
	"name=ADCL PGA Volume") control=gain ;;
	cget) verb=cget ;;
	cset) verb=cset ;;
	*) value="\$a" ;;
	esac
done
if [ "\$verb" = cget ]; then
	[ -e "$T/\$control" ] && echo "  : values=\$(cat "$T/\$control")"
	exit 0
fi
[ "\$control" = master ] && [ ! -e "$T/master" ] && exit 1
echo "\$value" >"$T/\$control"
EOF
# aplay: opening the volume's own device makes the control, as softvol does.
cat >"$T/bin/aplay" <<EOF
#!/bin/sh
echo "\$*" >>"$T/aplay.log"
case "\$*" in *catalyst_volume_init*) echo 100,100 >"$T/master" ;; esac
exit 0
EOF
cat >"$T/bin/modprobe" <<EOF
#!/bin/sh
echo "\$*" >>"$T/modprobe.log"
mkdir -p "$T/proc/ES8389Audio/pcm0p/sub0"
echo closed >"$T/proc/ES8389Audio/pcm0p/sub0/status"
EOF
cat >"$T/bin/bluealsa-cli" <<EOF
#!/bin/sh
cat "$T/bluealsa-pcms" 2>/dev/null
EOF
chmod +x "$T/bin/"*
PATH="$T/bin:$PATH"
export PATH
export CATALYST_AUDIO_STATE="$T/data" CATALYST_AUDIO_RUN="$T/run" CATALYST_AUDIO_SHARE="$T/share" \
	CATALYST_AUDIO_PROC="$T/proc" CATALYST_AUDIO_GPIOMEM="$T/no-gpiomem"
P="${CATALYST_TEST_SH:-sh} $HERE/os/board/cardputer-zero/rootfs-overlay/usr/sbin/catalyst-audio"

fail=0
check() {
	if [ "$2" = "$3" ]; then echo "   ok    $1"; else echo "   FAIL  $1: got [$2], want [$3]"; fail=1; fi
}
status_of() { $P status | sed -n "s/^$1=//p"; }

echo "== start"
$P start
check "the drivers, in order" "$(cat "$T/modprobe.log")" "-a snd-soc-bcm2835-i2s es8389_m5stack simple-amplifier cardputerzero-audio"
check "the volume control made before any sound" "$(grep -c catalyst_volume_init "$T/aplay.log")" "1"
check "and set to the level it starts at" "$(cat "$T/master")" "65,65"
check "the handheld's own microphone" "$(cat "$T/pgal")" "DifferentialL"
check "at M5Stack's gain" "$(cat "$T/gain")" "8"
check "ALSA's configuration in place" "$(cmp -s "$T/run/asound.conf" "$T/share/asound.conf" && echo same)" "same"
check "the speaker" "$(cat "$T/run/output")" "local"
$P start
check "a second start does not make the control again" "$(grep -c catalyst_volume_init "$T/aplay.log")" "1"

echo "== volume"
check "the level it starts at" "$($P volume)" "65"
check "set" "$($P volume 40):$(cat "$T/master"):$(cat "$T/data/audio-volume")" "40:40,40:40"
check "up" "$($P volume +10)" "50"
check "down past nothing stops at 0" "$($P volume -80)" "0"
check "past full stops at 100" "$($P volume 150)" "100"
check "a word is refused" "$($P volume loud 2>/dev/null || echo refused)" "refused"
$P volume 55 >/dev/null

echo "== mute"
check "on" "$($P mute on):$(cat "$T/master")" "on:0,0"
check "the level is kept" "$($P volume):$(status_of muted)" "55:yes"
check "toggled back" "$($P mute toggle):$(cat "$T/master")" "off:55,55"
$P mute on >/dev/null
check "turning it up unmutes" "$($P volume +5):$(cat "$T/master")" "60:60,60"
$P mute on >/dev/null
check "a level set while muted is not heard" "$($P volume 30):$(cat "$T/master"):$(status_of muted)" "30:0,0:yes"
check "until unmuted" "$($P mute off):$(cat "$T/master")" "off:30,30"

echo "== microphone"
$P mic headset
check "the headset's" "$(cat "$T/pgal"):$($P mic)" "Line 2P:headset"
$P mic builtin
check "and the handheld's own again" "$(cat "$T/pgal")" "DifferentialL"
check "a gain the codec takes" "$($P mic-gain 10):$(cat "$T/gain")" "10:10"
check "4 and 5 are not allowed: 6" "$($P mic-gain 5)" "6"
check "past 14 stops there" "$($P mic-gain 40)" "14"
check "nothing else" "$($P mic loud 2>/dev/null || echo refused)" "refused"

echo "== outputs"
check "the speaker, with nothing plugged in" "$(status_of output)" "speaker"
echo on >"$T/jack"
check "headphones, once plugged in" "$(status_of output)" "headphones"
rm "$T/jack"
check "an address that is not one is refused" "$($P output bluetooth nope 2>/dev/null || echo refused)" "refused"
$P output bluetooth 0c:a6:94:12:34:56
check "chosen, kept in capitals" "$($P output)" "bluetooth 0C:A6:94:12:34:56"
check "not connected: the speaker still plays" "$(cat "$T/run/output"):$(grep -c bluealsa "$T/run/asound.conf")" "local:0"
echo "/org/bluealsa/hci0/dev_0C_A6_94_12_34_56/a2dpsrc/sink" >"$T/bluealsa-pcms"
check "connected: Bluetooth in front" "$($P route)" "bluetooth"
check "to that device, over A2DP" "$(grep -c 'bluealsa:DEV=0C:A6:94:12:34:56,PROFILE=a2dp' "$T/run/asound.conf")" "1"
check "at the same volume control" "$(grep -c 'name "Master Playback Volume"' "$T/run/asound.conf")" "3"
check "the microphone stays the handheld's" "$(sed -n '/^# Bluetooth/,$p' "$T/run/asound.conf" | grep -c 'capture.pcm "catalyst_capture"')" "1"
check "status says so" "$(status_of output):$(status_of bluetooth)" "bluetooth:0C:A6:94:12:34:56"
rm "$T/bluealsa-pcms"
check "gone: back to the speaker" "$($P route)" "local"
$P output speaker
check "the speaker chosen again" "$($P output):$(status_of bluetooth)" "speaker:"

echo "== status"
check "the card" "$(status_of card)" "ES8389Audio"
check "nothing playing" "$(status_of playing)" "no"
echo "state: RUNNING" >"$T/proc/ES8389Audio/pcm0p/sub0/status"
check "something playing" "$(status_of playing)" "yes"
check "the amplifier unknown without /dev/gpiomem" "$($P amp)" "unknown"
rm -rf "$T/proc/ES8389Audio"
check "no card" "$(status_of card)" "none"
check "and nothing set on it" "$($P volume 70 >/dev/null; cat "$T/master")" "30,30"

exit $fail
