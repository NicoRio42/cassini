# The release binary is built on ubuntu-22.04 with a static glibc, which still
# loads the system's NSS modules for DNS: keep the same Ubuntu release here.
FROM ubuntu:22.04
# The OpenStreetMap download uses HTTPS.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY ./target/x86_64-unknown-linux-gnu/release/cassini /bin
ENTRYPOINT ["/bin/cassini"]
