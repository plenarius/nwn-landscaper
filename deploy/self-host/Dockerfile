# The Landscaper team server, built from source: `docker build -f deploy/self-host/Dockerfile -t landscaper .` from the
# repository's root.
#
#   --build-arg WEB_REPO=<git url of the editor>  (default plenarius/nwn-landscaper-web)
#   --build-arg WEB_REF=<the editor's tag or branch>  (default: the editor's release of this engine's version, below)
#   or put a copy of nwn-landscaper-web in this repository's root (./landscaper-web) and it is used instead of cloning
#   --build-arg NWN_NIM=2.3.2                     (the neverwinter.nim release that has nwn_gff)
#   --build-arg NWN_NIM_SHA256=<sha256>           (its x86_64 Linux zip's, as the release page lists it; change both together)

FROM rust:1-bookworm AS build
ARG WEB_REPO=https://github.com/plenarius/nwn-landscaper-web.git
# The editor of the same release as this engine: both repositories are tagged vX.Y.Z together, and this is `v` and the version
# in Cargo.toml (tests/release.rs fails when they disagree). `main` builds the editor's latest, which may not match.
ARG WEB_REF=v0.1.2
ARG NWN_NIM=2.3.2
ARG NWN_NIM_SHA256=f6c896f2821315acc70d2dffd2b8f86f17aa4fc2a549412864c1eab37aadcb19
RUN apt-get update && apt-get install -y --no-install-recommends cmake git ca-certificates curl unzip && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p landscaper
# The editor: a copy of nwn-landscaper-web in this repository's root if there is one (a private fork, or a version you are
# testing), else cloned.
RUN if [ -f /src/landscaper-web/index.html ]; then cp -r /src/landscaper-web /web; else git clone --depth 1 --branch "$WEB_REF" "$WEB_REPO" /web; fi && rm -rf /web/.git
RUN curl -sSfL -o /tmp/nim.zip "https://github.com/niv/neverwinter.nim/releases/download/${NWN_NIM}/neverwinter-x86_64-linux-gnu.zip" \
 && echo "${NWN_NIM_SHA256}  /tmp/nim.zip" | sha256sum -c - \
 && unzip -q -j /tmp/nim.zip nwn_gff -d /usr/local/bin && chmod +x /usr/local/bin/nwn_gff

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 curl && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 1000 landscaper && mkdir -p /data /nwn && chown landscaper /data
COPY --from=build /src/target/release/landscaper /usr/local/bin/landscaper
COPY --from=build /usr/local/bin/nwn_gff /usr/local/bin/nwn_gff
COPY --from=build /web /opt/landscaper/web
# Mounted: the game's files (NWN_ROOT, read only), your haks and talk table (NWN_HOME: hak/, tlk/) and the data folder
# (workspaces, the audit log, the texture library the first export builds).
ENV NWN_ROOT=/nwn/game NWN_HOME=/nwn/home XDG_DATA_HOME=/data LANDSCAPER_TEAM=1
USER landscaper
EXPOSE 8766
HEALTHCHECK --interval=30s --timeout=5s --retries=3 CMD curl -sf http://127.0.0.1:8766/healthz || exit 1
CMD ["landscaper", "serve", "--team", "--bind", "0.0.0.0", "--port", "8766", "--web", "/opt/landscaper/web"]
