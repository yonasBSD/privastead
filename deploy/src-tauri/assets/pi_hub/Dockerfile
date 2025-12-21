# SPDX-License-Identifier: GPL-3.0-or-later
FROM debian:bookworm

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      bash ca-certificates curl xz-utils jq \
      util-linux kmod \
      unzip \
      parted dosfstools e2fsprogs rsync \
    ; \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY build.sh /app/build.sh
ENTRYPOINT ["bash", "/app/build.sh"]
