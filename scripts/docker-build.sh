#!/bin/sh
# Build in a container (macOS, or any host without Buildroot's tools).
#
#   scripts/docker-build.sh [make targets...]    default: all
#   scripts/docker-build.sh --dist
#
# The finished image is copied to images/sdcard.img. It carries what is
# in the gitignored secrets/: Wi-Fi networks (*.nmconnection), SSH keys
# (authorized_keys) and files for the data partition (data/, e.g.
# data/quake3/baseq3/*.pk3). Such an image is for your own SD card only.
#
# --dist makes an image to give to others: a clean build of the committed
# sources only (git archive of HEAD: nothing from secrets/, no game data),
# plus Buildroot's legal-info (licences and the source code of everything
# in the image). This checkout must be clean. Output, in images/:
#   x6100-q3-<version>.img.xz             the image
#   x6100-q3-<version>-legal-info.tar     give it along with the image
#
# This checkout is mounted read-only. The build tree, the download cache
# and Buildroot itself live in Docker volumes (Buildroot needs a
# case-sensitive filesystem); their names can be changed:
#   X6100_OUTPUT_VOLUME     (default x6100-q3-output)
#   X6100_DL_VOLUME         (default x6100-q3-dl; can be shared with other
#                           Buildroot trees, files are checked by hash)
#   X6100_BUILDROOT_VOLUME  (default x6100-q3-buildroot)
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
output_volume=${X6100_OUTPUT_VOLUME:-x6100-q3-output}
dl_volume=${X6100_DL_VOLUME:-x6100-q3-dl}
br_volume=${X6100_BUILDROOT_VOLUME:-x6100-q3-buildroot}
image=x6100-q3-builder
src=$here
dist=0
if [ "${1:-}" = --dist ]; then
	shift
	[ $# -eq 0 ] || { echo "--dist takes no make targets" >&2; exit 1; }
	dist=1
	if [ -n "$(git -C "$here" status --porcelain)" ]; then
		echo "--dist: the checkout has uncommitted changes" >&2
		exit 1
	fi
	# Committed sources only: never anything from secrets/ or images/.
	src=$here/.work/dist/src
	rm -rf "$src"
	mkdir -p "$src"
	git -C "$here" archive HEAD | tar -x -C "$src"
	git -C "$here" rev-parse HEAD >"$src/.x6100-q3-commit"
	set -- clean legal-info all
fi
version=$(git -C "$here" describe --always --dirty 2>/dev/null || echo unknown)
name=x6100-q3-$version

docker build -q -t "$image" "$here/docker" >/dev/null
mkdir -p "$here/images"

tty=
[ -t 0 ] && tty=-it
[ $# -eq 0 ] && set -- all

# shellcheck disable=SC2086
docker run --rm $tty \
	-v "$src":/src/x6100-q3:ro \
	-v "$output_volume":/work/output \
	-v "$dl_volume":/work/dl \
	-v "$br_volume":/work/buildroot \
	-v "$here/images":/work/images \
	-e FORCE_UNSAFE_CONFIGURE=1 \
	-e X6100_DIST=$dist \
	-e X6100_VERSION="$version" \
	"$image" sh -c '
		set -e
		mk() { make -C /src/x6100-q3 O=/work/output DL=/work/dl BR_DIR=/work/buildroot "$@"; }
		# One make per target, in order (clean must be done before the
		# next target looks at the tree).
		for t in "$@"; do mk "$t"; done
		img=/work/output/images/sdcard.img
		[ -f $img ] || exit 0
		if [ "$X6100_DIST" = 1 ]; then
			cp $img /work/images/'"$name"'.img
			xz -T0 -9 -f /work/images/'"$name"'.img
			tar -C /work/output -cf /work/images/'"$name"'-legal-info.tar legal-info
			echo "images/'"$name"'.img.xz and images/'"$name"'-legal-info.tar"
		else
			cp $img /work/images/
			echo "copied to images/sdcard.img"
		fi
	' sh "$@"
