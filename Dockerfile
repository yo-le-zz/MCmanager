# MCManager en mode serveur : interface web multi-utilisateurs + controle a distance.
#   docker build -t mcmanager .
#   docker run -d --name mcmanager --restart unless-stopped -p 7777:7777 -p 25565:25565 -p 19132:19132/udp -v mcmanager-data:/data mcmanager
# --restart unless-stopped : apres une mise a jour (ou « Redemarrer MCManager »), le conteneur quitte avec le code 75 et
# Docker le relance ; les serveurs Minecraft qui tournaient repartent seuls.
# Le mot de passe admin initial s'affiche dans `docker logs mcmanager` (ou fixez
# MCMANAGER_ADMIN_USER / MCMANAGER_ADMIN_PASSWORD).

# ---- build ----
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY assets ./assets
RUN cargo build --release --bin mcmanager-server

# ---- runtime ----
# Java 21 (Minecraft 1.20.5+) comme JVM par defaut, Java 17 en plus pour les versions
# 1.17 a 1.20.4 : choisissez-la par serveur via "chemin Java" = /opt/java/17/bin/java.
FROM eclipse-temurin:21-jre-jammy
COPY --from=eclipse-temurin:17-jre-jammy /opt/java/openjdk /opt/java/17
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl tini libcurl4 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --home-dir /data --uid 10001 mcmanager
COPY --from=build /src/target/release/mcmanager-server /usr/local/bin/mcmanager-server
COPY web /app/web

ENV MCMANAGER_DATA_DIR=/data \
    MCMANAGER_HOST=0.0.0.0 \
    MCMANAGER_PORT=7777 \
    MCMANAGER_WEB_DIR=/app/web \
    MCMANAGER_NO_BROWSER=1
USER mcmanager
WORKDIR /data
VOLUME /data
# 7777 : interface web + controle a distance. Les ports des serveurs Minecraft sont ceux
# que vous creez : publiez-les avec -p (Java : 25565/tcp par defaut ; Bedrock : 19132/udp).
# Bedrock officiel : image amd64 seulement (le serveur de Mojang n'existe pas pour ARM) ;
# PocketMine-MP et Nukkit fonctionnent aussi en arm64.
EXPOSE 7777 25565 19132/udp
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD curl -fsS http://127.0.0.1:7777/healthz || exit 1
# tini relaie SIGTERM : MCManager arrete proprement les serveurs Minecraft au `docker stop`.
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/bin/tini", "--", "mcmanager-server"]
CMD ["--daemon"]
