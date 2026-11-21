#!/usr/bin/env python3
"""Test de bout en bout avec de VRAIS processus (aucune dépendance : bibliothèque standard uniquement).

Usage : python3 scripts/two-process-test.py --server target/debug/mcmanager-server --desktop target/debug/mcmanager

Scénarios :
  1. deux instances (serveur + bureau) jumelées par code, transfert d'un serveur (copie puis déplacement) et d'une
     sauvegarde par le tunnel chiffré, vérification de l'empreinte ;
  2. HTTPS natif (certificat auto-signé) : poignée de main, cookie Secure, redirection HTTP -> HTTPS ;
  3. migration : un dossier de données « 1.0.x » est repris, sauvegardé, complété, sans rien perdre ;
  4. reprise : un serveur Bedrock factice qui tournait est relancé après un redémarrage de MCManager,
     et arrêté proprement quand MCManager reçoit SIGTERM.
Sort en code 1 au premier écart.
"""
import argparse, http.cookiejar, io, json, os, shutil, signal, socket, ssl, subprocess, sys, tempfile, time, urllib.error, urllib.request, uuid, zipfile

ap = argparse.ArgumentParser()
ap.add_argument("--server", required=True); ap.add_argument("--desktop", required=True)
ap.add_argument("--web", default=os.path.join(os.path.dirname(__file__), "..", "web"))
args = ap.parse_args()
failures, procs, tmpdirs = [], [], []

def check(ok, msg):
    print(("ok   - " if ok else "FAIL - ") + msg, flush=True)
    if not ok: failures.append(msg)

def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0)); return s.getsockname()[1]

def start(binary, port, data, extra_env=None, extra_args=()):
    env = dict(os.environ, MCMANAGER_PORT=str(port), MCMANAGER_HOST="127.0.0.1", MCMANAGER_DATA_DIR=data, MCMANAGER_NO_BROWSER="1", MCMANAGER_WEB_DIR=os.path.abspath(args.web), RUST_LOG="warn")
    env.update(extra_env or {})
    p = subprocess.Popen([binary, *extra_args], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    procs.append(p); return p

def wait_http(url, ctx=None, tries=150):
    for _ in range(tries):
        try: urllib.request.urlopen(url, timeout=1.5, context=ctx); return True
        except urllib.error.HTTPError: return True
        except Exception: time.sleep(0.2)
    return False

class Client:
    def __init__(self, base, ctx=None):
        self.base, self.ctx, self.jar = base, ctx, http.cookiejar.CookieJar()
        handlers = [urllib.request.HTTPCookieProcessor(self.jar)] + ([urllib.request.HTTPSHandler(context=ctx)] if ctx else [])
        self.op = urllib.request.build_opener(*handlers)
    def call(self, method, path, body=None, headers=None, raw=False):
        data, h = None, dict(headers or {})
        if body is not None and not isinstance(body, bytes):
            data = json.dumps(body).encode(); h.setdefault("Content-Type", "application/json")
        elif body is not None: data = body
        h.setdefault("Origin", self.base)
        try:
            r = self.op.open(urllib.request.Request(self.base + path, data=data, method=method, headers=h), timeout=60)
            txt = r.read(); return r.status, (txt if raw else (json.loads(txt) if txt else {}))
        except urllib.error.HTTPError as e:
            txt = e.read()
            try: return e.code, json.loads(txt)
            except Exception: return e.code, txt.decode(errors="replace")
    def get(self, p): return self.call("GET", p)
    def post(self, p, b=None): return self.call("POST", p, b if b is not None else {})

def multipart(fields, file_field, filename, content):
    b = uuid.uuid4().hex; out = io.BytesIO()
    for k, v in fields.items(): out.write(f'--{b}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
    out.write(f'--{b}\r\nContent-Disposition: form-data; name="{file_field}"; filename="{filename}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode()); out.write(content); out.write(f"\r\n--{b}--\r\n".encode())
    return out.getvalue(), {"Content-Type": f"multipart/form-data; boundary={b}"}

def server_zip(world_bytes):
    z = io.BytesIO()
    with zipfile.ZipFile(z, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("server.jar", b"JAR"); zf.writestr("server.properties", "motd=Demo\nserver-port=25999\n"); zf.writestr("world/level.dat", world_bytes)
    return z.getvalue()

def poll(fn, tries=120, delay=0.5):
    for _ in range(tries):
        v = fn()
        if v: return v
        time.sleep(delay)
    return None

def stop_all():
    for p in procs:
        if p.poll() is None:
            p.terminate()
            try: p.wait(8)
            except Exception: p.kill()
    for d in tmpdirs: shutil.rmtree(d, ignore_errors=True)

try:
    # ───────── 1. Deux instances jumelées ─────────
    print("== 1. deux instances, transferts par le tunnel chiffré")
    da, db = tempfile.mkdtemp(prefix="mcm-A-"), tempfile.mkdtemp(prefix="mcm-B-"); tmpdirs += [da, db]
    pa, pb = free_port(), free_port()
    start(args.server, pa, da, {"MCMANAGER_ADMIN_USER": "admin", "MCMANAGER_ADMIN_PASSWORD": "motdepasse-test-1"}, ("--daemon",))
    start(args.desktop, pb, db)
    check(wait_http(f"http://127.0.0.1:{pa}/healthz") and wait_http(f"http://127.0.0.1:{pb}/healthz"), "les deux instances démarrent")
    A, B = Client(f"http://127.0.0.1:{pa}"), Client(f"http://127.0.0.1:{pb}")
    st, _ = A.post("/auth/login", {"username": "admin", "password": "motdepasse-test-1"})
    check(st == 200, "connexion à l'instance serveur")
    st, code = A.post("/api/server/pairing-code", {})
    check(st == 200 and code.get("code"), f"code de jumelage généré ({st})")
    st, r = B.post("/api/remote/targets", {"host": f"127.0.0.1:{pa}", "label": "vps", "code": code["code"]})
    check(st == 200, f"jumelage accepté {r if st != 200 else ''}")

    world = os.urandom(3 * 1024 * 1024 + 11)
    body, h = multipart({"name": "Demo", "loader": "paper", "mc_version": "1.21.4"}, "file", "demo.zip", server_zip(world))
    st, srv = B.call("POST", "/api/servers/import-zip", body, h)
    check(st == 200 and srv.get("id"), f"serveur de démonstration importé ({st})")
    sid = srv["id"]
    st, job = B.post("/api/transfers/start", {"op": "move_server", "server_id": sid, "label": "vps", "delete_source": False})
    jid = job.get("job")
    done = poll(lambda: next((j for j in B.get("/api/transfers")[1] if j["id"] == jid and j["state"] != "running"), None), 200, 0.3)
    check(done and done["state"] == "done", f"copie du serveur (3 Mo) vers l'autre machine : {done and done['state']} {done and done['message']}")
    st, la = A.get("/api/servers")
    remote_copy = next((s for s in la if s["name"] == "Demo"), None)
    check(remote_copy is not None, "le serveur existe sur l'instance serveur")
    check(any(s["id"] == sid for s in B.get("/api/servers")[1]), "copie : l'original est conservé")
    # Le fichier du monde est identique octet pour octet.
    st, raw = A.call("GET", f"/api/servers/{remote_copy['id']}/files/export?path=world", raw=True)
    got = None
    if st == 200:
        with zipfile.ZipFile(io.BytesIO(raw)) as zf:
            got = next((zf.read(n) for n in zf.namelist() if n.endswith("level.dat")), None)
    check(got == world, f"le monde reçu est identique à l'original octet pour octet ({st}, {len(got) if got else 0} octets)")

    st, _ = B.post(f"/api/servers/{sid}/backups")
    bk = poll(lambda: (B.get(f"/api/servers/{sid}/backups")[1] or [None])[0], 60, 0.5)
    check(bk is not None, "sauvegarde créée")
    st, job = B.post("/api/transfers/start", {"op": "send_backup", "server_id": sid, "name": bk["name"], "label": "vps"})
    jid = job.get("job")
    done = poll(lambda: next((j for j in B.get("/api/transfers")[1] if j["id"] == jid and j["state"] != "running"), None), 120, 0.3)
    check(done and done["state"] == "done", f"sauvegarde envoyée : {done and done['message']}")
    inc = A.get("/api/incoming-backups")[1]
    check(len(inc) == 1 and inc[0]["name"].endswith(bk["name"]), f"la sauvegarde est rangée côté serveur : {[i['name'] for i in inc]}")
    st, job = B.post("/api/transfers/start", {"op": "move_server", "server_id": sid, "label": "vps", "delete_source": True})
    jid = job.get("job")
    done = poll(lambda: next((j for j in B.get("/api/transfers")[1] if j["id"] == jid and j["state"] != "running"), None), 200, 0.3)
    check(done and done["state"] == "done" and "supprimé" in done["message"], f"déplacement : {done and done['message']}")
    check(not any(s["id"] == sid for s in B.get("/api/servers")[1] if not s.get("remote")), "déplacé : l'original a disparu de la machine d'origine")
    check(sum(1 for s in A.get("/api/servers")[1] if s["name"] == "Demo") == 2, "deux serveurs « Demo » sur l'instance serveur (copie + déplacement)")
    st, _ = B.post("/api/transfers/start", {"op": "move_server", "server_id": str(uuid.uuid4()), "label": "vps", "delete_source": True})
    j = poll(lambda: next((j for j in B.get("/api/transfers")[1] if j["state"] == "error"), None), 40, 0.3)
    check(j is not None and "introuvable" in j["message"], "un transfert impossible se termine en erreur claire, sans rien supprimer")

    # ───────── 2. HTTPS natif ─────────
    print("== 2. HTTPS natif")
    dc = tempfile.mkdtemp(prefix="mcm-C-"); tmpdirs.append(dc)
    pc, redir = free_port(), free_port()
    start(args.desktop, pc, dc, {"MCMANAGER_TLS": "self-signed", "MCMANAGER_HTTP_REDIRECT_PORT": str(redir)})
    lax = ssl.create_default_context(); lax.check_hostname = False; lax.verify_mode = ssl.CERT_NONE
    check(wait_http(f"https://127.0.0.1:{pc}/healthz", lax), "HTTPS répond (certificat auto-signé)")
    strict = ssl.create_default_context()
    try: urllib.request.urlopen(f"https://127.0.0.1:{pc}/healthz", timeout=3, context=strict); strict_ok = True
    except Exception: strict_ok = False
    check(not strict_ok, "un client strict refuse le certificat auto-signé (comportement normal)")
    r = urllib.request.urlopen(f"https://127.0.0.1:{pc}/api/version", timeout=5, context=lax)
    check(json.loads(r.read()).get("version"), "l'API fonctionne derrière TLS")
    check(os.path.isfile(os.path.join(dc, "tls", "cert.pem")) and os.path.isfile(os.path.join(dc, "tls", "key.pem")), "certificat créé dans <données>/tls/")
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *a, **k): return None
    try:
        urllib.request.build_opener(NoRedirect).open(f"http://127.0.0.1:{redir}/api/version", timeout=5); loc = None
    except urllib.error.HTTPError as e: loc = e.headers.get("Location")
    check(loc and loc.startswith("https://") and f":{pc}/api/version" in loc, f"HTTP redirige vers HTTPS ({loc})")
    # Instance protégée : le cookie de session porte l'attribut Secure.
    dd = tempfile.mkdtemp(prefix="mcm-D-"); tmpdirs.append(dd); pd = free_port()
    start(args.server, pd, dd, {"MCMANAGER_TLS": "self-signed", "MCMANAGER_ADMIN_USER": "admin", "MCMANAGER_ADMIN_PASSWORD": "motdepasse-test-1"}, ("--daemon",))
    check(wait_http(f"https://127.0.0.1:{pd}/healthz", lax), "instance serveur en HTTPS")
    req = urllib.request.Request(f"https://127.0.0.1:{pd}/auth/login", data=json.dumps({"username": "admin", "password": "motdepasse-test-1"}).encode(), headers={"Content-Type": "application/json", "Origin": f"https://127.0.0.1:{pd}"}, method="POST")
    r = urllib.request.urlopen(req, timeout=60, context=lax)
    check("Secure" in (r.headers.get("Set-Cookie") or ""), f"cookie de session Secure ({r.headers.get('Set-Cookie')})")

    # ───────── 3. Migration d'un ancien dossier de données ─────────
    print("== 3. migration depuis une ancienne version")
    de = tempfile.mkdtemp(prefix="mcm-E-"); tmpdirs.append(de)
    s1, s2 = str(uuid.uuid4()), str(uuid.uuid4())
    os.makedirs(os.path.join(de, "servers", s1)); open(os.path.join(de, "servers", s1, "world.dat"), "w").write("MONDE")
    legacy = {s1: {"id": s1, "name": "Survie 1.0", "loader": "paper", "mc_version": "1.20.4", "folder": os.path.join(de, "servers", s1), "jar_name": "paper.jar", "java_path": "java", "xms_mb": 1024, "xmx_mb": 4096, "port": 25565, "eula_accepted": True},
              s2: {"name": "Futur", "loader": "loader-inconnu", "mc_version": "9.9"}}
    open(os.path.join(de, "servers.json"), "w").write(json.dumps(legacy, indent=2))
    open(os.path.join(de, "config.json"), "w").write(json.dumps({"java_path": "/usr/bin/java"}))
    pe = free_port(); start(args.desktop, pe, de)
    check(wait_http(f"http://127.0.0.1:{pe}/healthz"), "l'instance démarre sur les anciennes données")
    E = Client(f"http://127.0.0.1:{pe}")
    srvs = E.get("/api/servers")[1]
    check(len(srvs) == 1 and srvs[0]["name"] == "Survie 1.0" and srvs[0]["xmx_mb"] == 4096, f"le serveur lisible est repris avec ses réglages : {[s['name'] for s in srvs]}")
    mig = E.get("/api/migration")[1]
    check(mig["schema"] == mig["current"] and mig["rejected_servers"] == 1 and len(mig["backups"]) == 1, f"migration enregistrée : {mig}")
    check(os.path.isfile(os.path.join(de, "servers.rejected.json")), "l'entrée illisible est mise de côté, pas supprimée")
    bdir = os.path.join(de, "migration-backups", mig["backups"][0])
    check(json.load(open(os.path.join(bdir, "servers.json"))) == legacy, "copie de sécurité identique à l'original")
    check(open(os.path.join(de, "servers", s1, "world.dat")).read() == "MONDE", "le monde n'a pas été touché")

    # ───────── 4. Reprise des serveurs après redémarrage ─────────
    print("== 4. reprise automatique et arrêt propre")
    df = tempfile.mkdtemp(prefix="mcm-F-"); tmpdirs.append(df)
    sid = str(uuid.uuid4()); folder = os.path.join(df, "servers", sid); os.makedirs(folder)
    marker = os.path.join(folder, "stopped.txt")
    fake = f"""#!/bin/sh
echo "[INFO] Server started."
while read line; do
  case "$line" in
    stop) echo "arrete" > "{marker}"; echo "[INFO] Quit correctly"; exit 0 ;;
  esac
done
"""
    open(os.path.join(folder, "bedrock_server"), "w").write(fake); os.chmod(os.path.join(folder, "bedrock_server"), 0o644)
    open(os.path.join(folder, "server.properties"), "w").write("server-name=Fake\nserver-port=19999\nlevel-name=Bedrock level\n")
    json.dump({sid: {"id": sid, "name": "Bedrock factice", "loader": "bedrock", "mc_version": "1.21.50.07", "folder": folder, "jar_name": "bedrock_server", "java_path": "", "xms_mb": 512, "xmx_mb": 1024, "port": 19999, "eula_accepted": True}}, open(os.path.join(df, "servers.json"), "w"))
    json.dump({"servers": [sid]}, open(os.path.join(df, "running-servers.json"), "w"))
    pf = free_port(); proc = start(args.desktop, pf, df)
    F = Client(f"http://127.0.0.1:{pf}")
    check(wait_http(f"http://127.0.0.1:{pf}/healthz"), "MCManager démarre")
    running = poll(lambda: F.get(f"/api/servers/{sid}/status")[1].get("running"), 40, 0.5)
    check(bool(running), "le serveur qui tournait avant l'arrêt est relancé tout seul")
    proc.send_signal(signal.SIGTERM)
    try: proc.wait(30); exited = True
    except Exception: exited = False
    check(exited, "MCManager s'arrête sur SIGTERM")
    check(os.path.isfile(marker) and open(marker).read().strip() == "arrete", "le serveur a reçu « stop » avant l'arrêt (arrêt propre, le monde est sauvegardé)")
    check(json.load(open(os.path.join(df, "running-servers.json")))["servers"] == [sid], "il reste au manifeste : il reprendra au prochain démarrage")
    os.remove(marker)
    pf2 = free_port(); start(args.desktop, pf2, df); F2 = Client(f"http://127.0.0.1:{pf2}")
    check(wait_http(f"http://127.0.0.1:{pf2}/healthz"), "MCManager redémarre")
    check(bool(poll(lambda: F2.get(f"/api/servers/{sid}/status")[1].get("running"), 40, 0.5)), "le serveur reprend après le redémarrage")
    st, _ = F2.post(f"/api/servers/{sid}/stop")
    check(bool(poll(lambda: not F2.get(f"/api/servers/{sid}/status")[1].get("running"), 40, 0.5)), "arrêt voulu par l'utilisateur")
    time.sleep(0.5)
    check(json.load(open(os.path.join(df, "running-servers.json")))["servers"] == [], "un arrêt voulu le retire du manifeste : pas de relance surprise")
finally:
    stop_all()

print(f"\n{'ÉCHEC : ' + str(len(failures)) + ' vérification(s)' if failures else 'Tous les tests à plusieurs processus passent'}")
sys.exit(1 if failures else 0)
