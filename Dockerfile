# The build context is a directory holding the Linux `cassini` binary, e.g.
# the extracted cassini-x86_64-unknown-linux-gnu release archive:
#
#     docker build -f Dockerfile <directory with the binary>
#
# The release binary is built on ubuntu-22.04 and needs glibc 2.35 or later.
FROM ubuntu:22.04
# The OpenStreetMap download uses HTTPS.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# The image is upgraded by pulling a newer one: turns off the check for a newer
# version, and makes `cassini upgrade` say so.
ENV CASSINI_DOCKER=1
WORKDIR /app
COPY cassini /bin/cassini
ENTRYPOINT ["/bin/cassini"]
