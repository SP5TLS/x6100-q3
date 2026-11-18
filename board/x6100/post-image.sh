#!/bin/sh
# Buildroot post-image script: assemble sdcard.img.
set -eu
BOARD=$(dirname "$0")
CFG=$BOARD/genimage.cfg

# Files to preload into the x6100-data partition (/data on the radio) from
# the gitignored secrets/data/, e.g. quake3/ with game data that must not
# go into images for others: left out with X6100_DIST=1. The partition is
# then made big enough for them plus the usual 256 MiB of free space.
PRELOAD=$BR2_EXTERNAL_X6100_PATH/secrets/data
if [ "${X6100_DIST:-0}" != 1 ] && [ -d "$PRELOAD" ] && [ -n "$(ls -A "$PRELOAD")" ]; then
	used=$(du -sm "$PRELOAD" | cut -f1)
	size=$((used + used / 20 + 256))
	rm -f "$BINARIES_DIR/data.ext4"
	"$HOST_DIR/sbin/mke2fs" -q -F -t ext4 -L x6100-data -E root_owner=0:0 \
		-d "$PRELOAD" "$BINARIES_DIR/data.ext4" "${size}M"
	# genimage takes that data.ext4 as it is: drop its recipe for an empty one.
	CFG=$BINARIES_DIR/genimage-preload.cfg
	sed '/^image data\.ext4 {/,/^}/d' "$BOARD/genimage.cfg" >"$CFG"
	echo "x6100-data: $size MiB, preloaded with: $(ls "$PRELOAD" | tr '\n' ' ')"
fi

exec support/scripts/genimage.sh -c "$CFG"
