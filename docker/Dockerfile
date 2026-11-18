# Build environment for x6100-q3 (Buildroot's host requirements).
#
# Pinned, so a new Debian image or package update cannot change or break
# the build: the base image by date and digest, and its packages from
# snapshot.debian.org at the moment that image was made (the time is in
# the image's /etc/apt/sources.list.d/debian.sources). To move on, take
# a newer debian:trixie-YYYYMMDD-slim, its digest
# (`docker buildx imagetools inspect debian:trixie-YYYYMMDD-slim`) and the
# snapshot time from that file, and change all three together.
FROM debian:trixie-20261104-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

ARG SNAPSHOT=20261104T212339Z
ARG DEBIAN_FRONTEND=noninteractive
# Snapshot Release files are past their Valid-Until date by design; apt
# still checks every file against the archive's signatures.
RUN sed -i \
        -e "s|^URIs: http://deb.debian.org/debian-security\$|URIs: http://snapshot.debian.org/archive/debian-security/${SNAPSHOT}|" \
        -e "s|^URIs: http://deb.debian.org/debian\$|URIs: http://snapshot.debian.org/archive/debian/${SNAPSHOT}|" \
        /etc/apt/sources.list.d/debian.sources \
    && grep -q "snapshot.debian.org/archive/debian/${SNAPSHOT}" /etc/apt/sources.list.d/debian.sources \
    && printf 'Acquire::Check-Valid-Until "false";\nAcquire::Retries "5";\n' \
        > /etc/apt/apt.conf.d/99snapshot \
    && apt-get update && apt-get install -y --no-install-recommends \
        bc bison build-essential bzip2 ca-certificates cpio file flex g++ \
        gawk git gzip libncurses-dev locales make patch perl python3 \
        python3-dev python3-setuptools rsync sed unzip wget xz-utils \
    && sed -i 's/^# *en_US.UTF-8/en_US.UTF-8/' /etc/locale.gen && locale-gen \
    && rm -rf /var/lib/apt/lists/*

ENV LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
# The checkout is bind-mounted from the host, owned by another uid.
RUN git config --system --add safe.directory "*"
# Buildroot refuses to run as root unless told otherwise:
# scripts/docker-build.sh sets FORCE_UNSAFE_CONFIGURE=1.
WORKDIR /work
