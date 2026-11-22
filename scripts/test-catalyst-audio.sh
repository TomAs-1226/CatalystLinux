#!/bin/sh
# catalyst-audio against a stand-in sound card: amixer, aplay, modprobe and
# bluealsa-cli are small scripts that keep what they were asked in a scratch
# folder, and /proc/asound and /data are folders there too. Checks the
# volume (saved, clamped, muted and back), the microphone's choices, ALSA's
# configuration for the speaker and for Bluetooth headphones (only while
# connected), USB headphones plugged in and pulled out (mdev's hotplug, on
# their own control or a software one, their own volume, the one connected
# last of them and Bluetooth, deep standby's coming and going), mdev's rule,
# the status lines, and start (drivers loaded, volume control made before
# any sound, both restored). Run anywhere with a POSIX shell, sed and awk
# (WSL is fine): sh scripts/test-catalyst-audio.sh
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/data" "$T/run" "$T/proc" "$T/share/sounds"
cp "$HERE/os/board/cardputer-zero/rootfs-overlay/usr/share/catalyst/asound.conf" "$T/share/"
: >"$T/share/sounds/chime.wav"

# amixer: cget answers from files ("master" once the control exists, "jack"
# for the headphones); cset keeps what it was given. Asked about another
# card (-c, a USB one), it has the simple controls in usb-controls (a
# NAME|CAPABILITIES line each), and keeps what sset was last given.
cat >"$T/bin/amixer" <<EOF
#!/bin/sh
echo "\$*" >>"$T/amixer.log"
card=ES8389Audio
prev=
for a in "\$@"; do
	[ "\$prev" = -c ] && card=\$a
	prev=\$a
done
if [ "\$card" != ES8389Audio ]; then
	case "\$*" in
	*scontrols*) sed "s/^\([^|]*\)|.*/Simple mixer control '\1',0/" "$T/usb-controls" 2>/dev/null ;;
	*sget*)
		name=\${prev%,0}
		echo "Simple mixer control '\$name',0"
		echo "  Capabilities: \$(sed -n "s/^\$name|//p" "$T/usb-controls")"
		;;
	*sset*) echo "\$card \$*" >"$T/usb-level" ;;
	esac
	exit 0
fi
for a in "\$@"; do
	case "\$a" in
	"name=Master Playback Volume") control=master ;;
	"name=USB Playback Volume") control=usbvol ;;
	"name=Headphones Jack") control=jack ;;
	"name=PGAL Select") control=pgal ;;
	"name=ADCL PGA Volume") control=gain ;;
	"name=IF DACL Mixer DACR DACL Mixer") control=mix ;;
	"name=DACL Playback Volume") control=dacl ;;
	controls) verb=controls ;;
	cget) verb=cget ;;
	cset) verb=cset ;;
	*) value="\$a" ;;
	esac
done
if [ "\$verb" = controls ]; then
	printf "numid=31,iface=MIXER,name='DACL Playback Volume'\\nnumid=52,iface=MIXER,name='IF DACL Mixer DACR DACL Mixer'\\n"
	exit 0
fi
if [ "\$verb" = cget ]; then
	[ -e "$T/\$control" ] && echo "  : values=\$(cat "$T/\$control")"
	exit 0
fi
[ "\$control" = master ] && [ ! -e "$T/master" ] && exit 1
[ "\$control" = usbvol ] && [ ! -e "$T/usbvol" ] && exit 1
# Where sound went as the USB volume was set (it must be set first).
[ "\$control" = usbvol ] && cat "$T/run/output" >"$T/usbvol-while" 2>/dev/null
echo "\$value" >"$T/\$control"
EOF
# aplay: opening a volume's own device makes its control, as softvol does
# (at full volume).
cat >"$T/bin/aplay" <<EOF
#!/bin/sh
echo "\$*" >>"$T/aplay.log"
case "\$*" in
*catalyst_volume_init*) echo 100,100 >"$T/master" ;;
*catalyst_usb_volume_init*) echo 100,100 >"$T/usbvol" ;;
esac
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
case "\$1" in
info) cat "$T/bluealsa-info" 2>/dev/null ;;
*) cat "$T/bluealsa-pcms" 2>/dev/null ;;
esac
EOF
chmod +x "$T/bin/"*
PATH="$T/bin:$PATH"
export PATH
export CATALYST_AUDIO_STATE="$T/data" CATALYST_AUDIO_RUN="$T/run" CATALYST_AUDIO_SHARE="$T/share" \
	CATALYST_AUDIO_PROC="$T/proc" CATALYST_AUDIO_GPIOMEM="$T/no-gpiomem" CATALYST_AUDIO_DEEP="$T/deep"
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
check "the speaker plays both channels, 6 dB down" "$(cat "$T/mix"):$(cat "$T/dacl")" "on:179"
check "status says so" "$(status_of speaker_mix)" "on"
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
check "each ear its own channel, at 0 dB" "$($P speaker):$(cat "$T/dacl")" "speaker_mix=off:191"
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
printf 'Device: /org/bluez/hci0/dev_0C_A6_94_12_34_56\nSelected codec: AAC\nSampling: 48000 Hz\n' >"$T/bluealsa-info"
$P route >/dev/null
check "its codec and rate, for the player and the pages" "$(sed -n 's/^codec=//p' "$T/run/bluetooth"):$(sed -n 's/^rate=//p' "$T/run/bluetooth")" "AAC:48000"
check "status says the codec" "$(status_of bluetooth_codec):$(status_of bluetooth_rate)" "AAC:48000"
rm "$T/bluealsa-pcms"
check "gone: back to the speaker" "$($P route)" "local"
check "and its stream no longer described" "$([ -e "$T/run/bluetooth" ] && echo there || echo gone)" "gone"
$P output speaker
check "the speaker chosen again" "$($P output):$(status_of bluetooth)" "speaker:"

# A USB sound card as the kernel shows one: its folder under /proc/asound
# (id, usbid, a playback device) and its line in "cards".
plug() { # NUMBER ID USBID NAME
	mkdir -p "$T/proc/card$1/pcm0p" "$T/proc/card$1/pcm0c"
	echo "$2" >"$T/proc/card$1/id"
	echo "$3" >"$T/proc/card$1/usbid"
	printf ' %s [%-15s]: USB-Audio - %s\n                      %s at usb-3f980000.usb-1.2, full speed\n' "$1" "$2" "$4" "$4" >>"$T/proc/cards"
}
unplug() {
	rm -rf "$T/proc/card$1"
	sed -i "/^ *$1 \[/,+1d" "$T/proc/cards"
}
hotplug() { # add|remove NUMBER
	ACTION=$1 MDEV=snd/controlC$2 CATALYST_AUDIO_WAIT=1 $P hotplug
}
usb_part() { sed -n '/^# USB headphones/,$p' "$T/run/asound.conf"; }

echo "== USB headphones with a volume of their own"
echo "PCM|pvolume pswitch pswitch-joined" >"$T/usb-controls"
echo "Mic|cvolume cswitch" >>"$T/usb-controls"
plug 1 Adapter 05ac:110a "USB-C to 3.5mm Headphone Jack Adapter"
hotplug add 1
check "plugged in: they play" "$(cat "$T/run/output")" "usb 1"
check "to that card, mixed" "$(usb_part | grep -c 'pcm "hw:CARD=Adapter,DEV=0"'):$(usb_part | grep -c 'type dmix')" "1:1"
check "on their own control, no software volume" "$(usb_part | grep -c softvol)" "0"
check "at their own level, set as they came" "$(cat "$T/usb-level")" "Adapter -q -M -c Adapter sset PCM,0 40% unmute"
check "the microphone stays the handheld's" "$(usb_part | grep -c 'capture.pcm "catalyst_capture"')" "1"
check "status says so" "$(status_of output):$(status_of usb):$(status_of usb_mixer):$(status_of volume_usb)" "usb:Adapter:own:40"
check "and what they are" "$(status_of usb_name)" "USB-C to 3.5mm Headphone Jack Adapter"
check "the speaker's volume is its own" "$($P volume main):$(cat "$T/master")" "30:30,30"
check "the volume keys move theirs" "$($P volume +5):$(cat "$T/data/audio-volume-usb")" "45:45"
check "on their control" "$(cat "$T/usb-level")" "Adapter -q -M -c Adapter sset PCM,0 45% unmute"
check "and leave the speaker's" "$($P volume main):$(cat "$T/master")" "30:30,30"
check "the speaker's set by name, theirs kept" "$($P volume 50 main):$(cat "$T/master"):$($P volume usb)" "50:50,50:45"
$P volume 30 main >/dev/null
check "muted: theirs off too" "$($P mute on):$(cat "$T/usb-level")" "on:Adapter -q -M -c Adapter sset PCM,0 0% mute"
check "and back at their level" "$($P mute off):$(cat "$T/usb-level")" "off:Adapter -q -M -c Adapter sset PCM,0 45% unmute"
check "a second look changes nothing" "$($P route):$(ls "$T/run" | grep -c '\.[0-9]')" "usb 1:0"

echo "== USB and Bluetooth: the one connected last plays"
$P output bluetooth 0C:A6:94:12:34:56
echo "/org/bluealsa/hci0/dev_0C_A6_94_12_34_56/a2dpsrc/sink" >"$T/bluealsa-pcms"
check "Bluetooth connected after: Bluetooth" "$($P route)" "bluetooth"
check "status says both are there" "$(status_of output):$(status_of usb):$(status_of bluetooth_connected)" "bluetooth:Adapter:yes"
check "the USB ones chosen instead" "$($P prefer usb)" "usb 1"
check "and kept so at the next look" "$($P route)" "usb 1"
check "Bluetooth chosen again" "$($P prefer bluetooth)" "bluetooth"
check "a word is refused" "$($P prefer speaker 2>/dev/null || echo refused)" "refused"
unplug 1
hotplug remove 1
check "the USB ones out: Bluetooth plays on" "$(cat "$T/run/output"):$(status_of usb)" "bluetooth:"
plug 1 Adapter 05ac:110a "USB-C to 3.5mm Headphone Jack Adapter"
hotplug add 1
check "plugged in again, after Bluetooth: theirs" "$(cat "$T/run/output")" "usb 1"
rm "$T/bluealsa-pcms"
check "Bluetooth gone: still theirs" "$($P route)" "usb 1"

echo "== deep standby"
echo "/org/bluealsa/hci0/dev_0C_A6_94_12_34_56/a2dpsrc/sink" >"$T/bluealsa-pcms"
$P prefer bluetooth >/dev/null
echo "usb 3f980000.usb" >"$T/deep"
unplug 1
hotplug remove 1
check "the USB host stopped: Bluetooth" "$(cat "$T/run/output")" "bluetooth"
plug 1 Adapter 05ac:110a "USB-C to 3.5mm Headphone Jack Adapter"
rm "$T/deep"
hotplug add 1
check "back from it: still Bluetooth, as chosen" "$(cat "$T/run/output")" "bluetooth"
unplug 1
hotplug remove 1
plug 1 Adapter 05ac:110a "USB-C to 3.5mm Headphone Jack Adapter"
hotplug add 1
check "pulled out and back by hand: theirs" "$(cat "$T/run/output")" "usb 1"
rm "$T/bluealsa-pcms"
$P output speaker
unplug 1
hotplug remove 1
check "out: the speaker" "$(cat "$T/run/output"):$(status_of output):$(ls "$T/run/usb" 2>/dev/null)" "local:speaker:"
check "and the speaker's configuration whole" "$(cmp -s "$T/run/asound.conf" "$T/share/asound.conf" && echo same)" "same"

echo "== USB headphones with no volume of their own"
echo "Mic|cvolume cswitch" >"$T/usb-controls"
rm -f "$T/usb-level"
plug 2 Device 0d8c:0014 "USB Audio Device"
hotplug add 2
check "they play" "$(cat "$T/run/output")" "usb 2"
check "through a software volume" "$(usb_part | grep -c 'type softvol'):$(usb_part | grep -c 'name "USB Playback Volume"')" "1:1"
check "on the codec's card, which stays" "$(usb_part | sed -n '/USB Playback Volume/{n;p}' | tr -d '\t')" "card ES8389Audio"
check "made before any sound, and set" "$(grep -c catalyst_usb_volume_init "$T/aplay.log"):$(cat "$T/usbvol")" "1:45,45"
check "before sound went to them" "$(cat "$T/usbvol-while")" "local"
check "their own card's control left alone" "$(ls "$T/usb-level" 2>/dev/null)" ""
check "status says so" "$(status_of usb):$(status_of usb_mixer)" "Device:software"
check "a mixer of their own for each card" "$(usb_part | grep -c 'ipc_key 51832')" "1"
check "the volume keys move theirs" "$($P volume -15):$(cat "$T/usbvol")" "30:30,30"
check "a card named oddly is used by its number" "$(unplug 2; plug 2 'Odd-name' 0d8c:0015 x; hotplug add 2; usb_part | grep -c 'hw:CARD=2,DEV=0')" "1"
unplug 2
hotplug remove 2
check "a USB card that only records is not headphones" "$(mkdir -p "$T/proc/card3"; echo 046d:0825 >"$T/proc/card3/usbid"; echo Webcam >"$T/proc/card3/id"; $P route)" "local"
rm -rf "$T/proc/card3"

echo "== mdev's rule"
MDEV_CONF="$HERE/os/board/cardputer-zero/rootfs-overlay/etc/mdev.conf"
rule=$(grep -n 'catalyst-audio hotplug' "$MDEV_CONF" | cut -d: -f1)
plain=$(grep -n '^snd/control' "$MDEV_CONF" | tail -n1 | cut -d: -f1)
check "a sound card's control device runs it, coming and going" "$(grep -c '^snd/controlC\[0-9\]+[[:space:]]*root:audio 660 \*/usr/sbin/catalyst-audio hotplug$' "$MDEV_CONF")" "1"
check "which matches the card's name as mdev sees it" "$(echo snd/controlC1 | grep -cEx 'snd/controlC[0-9]+')" "1"
check "before the plain rule (mdev takes the first that matches)" "$([ "$rule" -lt "$plain" ] && echo yes)" "yes"
check "drivers still load as devices come" "$(grep -c '^\$MODALIAS=\.\*.*@modprobe "\$MODALIAS"$' "$MDEV_CONF")" "1"

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
