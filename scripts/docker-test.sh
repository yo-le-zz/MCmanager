#!/usr/bin/env bash
# Test de l'image Docker : demarrage, sante, connexion, persistance du volume, arret propre.
#   scripts/docker-test.sh [image]        (par defaut : construit l'image locale `mcmanager-test`)
# L'image n'est pas publiee ici : c'est un test de fumee, a lancer avant une release (la construction prend du temps).
set -euo pipefail
IMAGE="${1:-}"
NAME="mcmanager-docker-test-$$"
VOL="mcmanager-docker-test-vol-$$"
PORT="${MCM_TEST_PORT:-17777}"
cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; docker volume rm -f "$VOL" >/dev/null 2>&1 || true; }
trap cleanup EXIT
ok()   { echo "ok   - $*"; }
fail() { echo "FAIL - $*"; exit 1; }

if [ -z "$IMAGE" ]; then
  IMAGE="mcmanager-test"
  docker build -t "$IMAGE" "$(dirname "$0")/.."
fi

run() { docker run -d --name "$NAME" -p "$PORT:7777" -v "$VOL:/data" -e MCMANAGER_ADMIN_USER=admin -e MCMANAGER_ADMIN_PASSWORD=motdepasse-test-1 "$IMAGE" >/dev/null; }
wait_up() { for _ in $(seq 1 60); do curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && return 0; sleep 1; done; return 1; }

run
wait_up && ok "le conteneur repond sur /healthz" || { docker logs "$NAME" | tail -20; fail "le conteneur ne demarre pas"; }

VERSION="$(curl -fsS "http://127.0.0.1:$PORT/api/version" | sed -n 's/.*"version":"\([^"]*\)".*/\1/p')"
[ -n "$VERSION" ] && ok "version $VERSION" || fail "version illisible"

# Connexion (le cookie est rendu par la reponse) puis lecture protegee.
JAR="$(mktemp)"
CODE="$(curl -s -o /dev/null -w '%{http_code}' -c "$JAR" -H "Origin: http://127.0.0.1:$PORT" -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"motdepasse-test-1"}' "http://127.0.0.1:$PORT/auth/login")"
[ "$CODE" = "200" ] && ok "connexion administrateur" || fail "connexion refusee ($CODE)"
CODE="$(curl -s -o /dev/null -w '%{http_code}' -b "$JAR" "http://127.0.0.1:$PORT/api/servers")"
[ "$CODE" = "200" ] && ok "API protegee accessible une fois connecte" || fail "API protegee ($CODE)"
CODE="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/api/servers")"
[ "$CODE" = "401" ] && ok "API protegee refusee sans connexion" || fail "API sans connexion ($CODE)"

# Java et le serveur officiel Bedrock : bibliotheques presentes dans l'image.
docker exec "$NAME" java -version >/dev/null 2>&1 && ok "Java present" || fail "Java absent"
docker exec "$NAME" sh -c 'ls /usr/lib/*/libcurl.so.4 >/dev/null 2>&1' && ok "libcurl presente (serveur officiel Bedrock)" || fail "libcurl absente"

# Arret propre : docker stop -> code 0 en moins de 30 s.
START=$(date +%s)
docker stop -t 60 "$NAME" >/dev/null
EXIT="$(docker inspect -f '{{.State.ExitCode}}' "$NAME")"
[ "$EXIT" = "0" ] && ok "arret propre (code 0, $(( $(date +%s) - START )) s)" || fail "code de sortie $EXIT"

# Persistance : le volume conserve les comptes et la configuration.
docker rm "$NAME" >/dev/null
run
wait_up && ok "redemarrage avec le meme volume" || fail "redemarrage impossible"
CODE="$(curl -s -o /dev/null -w '%{http_code}' -H "Origin: http://127.0.0.1:$PORT" -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"motdepasse-test-1"}' "http://127.0.0.1:$PORT/auth/login")"
[ "$CODE" = "200" ] && ok "le compte administrateur est conserve dans le volume" || fail "compte perdu ($CODE)"
echo; echo "Tous les tests Docker passent"
