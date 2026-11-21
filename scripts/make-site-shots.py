#!/usr/bin/env python3
"""Captures d'écran du VRAI MCManager (données de démonstration) pour le site vitrine : website/assets/*.webp.

  python3 scripts/make-site-shots.py --binary target/debug/mcmanager --firefox <firefox> --geckodriver <geckodriver>

Les serveurs de démonstration sont de faux programmes (scripts) qui se comportent comme un serveur prêt : aucune
installation de Java ni de Minecraft n'est nécessaire. Nécessite : selenium, Pillow.
"""
import argparse, io, json, os, shutil, socket, stat, subprocess, tempfile, time, urllib.request, uuid

from PIL import Image
from selenium import webdriver
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service

ap = argparse.ArgumentParser()
ap.add_argument("--binary", required=True); ap.add_argument("--firefox", required=True); ap.add_argument("--geckodriver", required=True)
ap.add_argument("--web", default=os.path.join(os.path.dirname(__file__), "..", "web"))
ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "website", "assets"))
args = ap.parse_args()

def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0)); return s.getsockname()[1]

data = tempfile.mkdtemp(prefix="mcm-shots-")
FAKE = '#!/bin/sh\necho "Done (2.1s)! For help, type \\"help\\" or \\"?\\""\nwhile read l; do case "$l" in stop) exit 0;; esac; done\n'
FAKE_BE = '#!/bin/sh\necho "[INFO] Server started."\nwhile read l; do case "$l" in stop) exit 0;; esac; done\n'
FAKEJAVA = os.path.join(data, "fakejava.sh")
open(FAKEJAVA, "w").write(FAKE); os.chmod(FAKEJAVA, 0o755)   # ignore ses arguments (-Xmx, -jar…) et se comporte comme un serveur prêt
def mk(name, loader, ver, port, script, java=FAKEJAVA, jar="x.sh", extra=None):
    sid = str(uuid.uuid4()); folder = os.path.join(data, "servers", sid); os.makedirs(folder)
    open(os.path.join(folder, jar), "w").write(script); os.chmod(os.path.join(folder, jar), 0o755)
    open(os.path.join(folder, "server.properties"), "w").write(f"server-port={port}\nmotd={name}\nlevel-name={'Bedrock level' if loader.startswith('bedrock') else 'world'}\nmax-players=20\ngamemode=survival\ndifficulty=normal\n")
    e = {"id": sid, "name": name, "loader": loader, "mc_version": ver, "folder": folder, "jar_name": jar, "java_path": java, "xms_mb": 1024, "xmx_mb": 4096, "port": port, "eula_accepted": True,
         "auto_backup_minutes": 60, "auto_restart": True}
    e.update(extra or {}); return sid, e, folder
servers, ids = {}, []
for spec in [("Survie entre amis", "paper", "1.21.4", 25565, FAKE), ("Modpack Fabric", "fabric", "1.21.1", 25566, FAKE), ("Bedrock — Famille", "bedrock", "1.21.50.07", 19132, FAKE_BE, "", "bedrock_server"), ("Créatif (arrêté)", "purpur", "1.20.6", 25567, FAKE)]:
    sid, e, folder = mk(*spec[:5], *( [spec[5], spec[6]] if len(spec) > 5 else [] )); servers[sid] = e; ids.append((sid, folder))
bed_id, bed_folder = ids[2]
# Un add-on Bedrock installé et actif, pour la capture « Packs & mondes ».
bp = os.path.join(bed_folder, "behavior_packs", "Mobs_supplementaires_aaaaaaaa"); os.makedirs(bp)
open(os.path.join(bp, "manifest.json"), "w").write(json.dumps({"format_version": 2, "header": {"name": "Mobs supplémentaires", "description": "Ajoute 12 créatures", "uuid": "11111111-1111-4111-8111-111111111111", "version": [1, 4, 0]}, "modules": [{"type": "data", "uuid": "22222222-2222-4222-8222-222222222222", "version": [1, 0, 0]}]}))
os.makedirs(os.path.join(bed_folder, "worlds", "Bedrock level", "db")); open(os.path.join(bed_folder, "worlds", "Bedrock level", "level.dat"), "wb").write(os.urandom(200000))
os.makedirs(os.path.join(bed_folder, "worlds", "Aventure")); open(os.path.join(bed_folder, "worlds", "Aventure", "level.dat"), "wb").write(os.urandom(90000))
os.makedirs(os.path.join(bed_folder, "worlds", "Bedrock level"), exist_ok=True)
open(os.path.join(bed_folder, "worlds", "Bedrock level", "world_behavior_packs.json"), "w").write(json.dumps([{"pack_id": "11111111-1111-4111-8111-111111111111", "version": [1, 4, 0]}]))
json.dump(servers, open(os.path.join(data, "servers.json"), "w"))
json.dump({"servers": [ids[0][0], ids[1][0], ids[2][0]]}, open(os.path.join(data, "running-servers.json"), "w"))
json.dump({"theme": "graphite"}, open(os.path.join(data, "ui.json"), "w"))

port = free_port()
env = dict(os.environ, MCMANAGER_PORT=str(port), MCMANAGER_HOST="127.0.0.1", MCMANAGER_DATA_DIR=data, MCMANAGER_NO_BROWSER="1", MCMANAGER_WEB_DIR=os.path.abspath(args.web))
proc = subprocess.Popen([args.binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
driver = None
try:
    for _ in range(100):
        try: urllib.request.urlopen(f"http://127.0.0.1:{port}/healthz", timeout=1); break
        except Exception: time.sleep(0.2)
    time.sleep(8)  # reprise des serveurs de démonstration (3 s d'écart)
    o = Options(); o.add_argument("-headless"); o.binary_location = args.firefox
    driver = webdriver.Firefox(options=o, service=Service(args.geckodriver))
    def shot(name, w, h, scale=1):
        driver.set_window_size(w, h); time.sleep(1.2)
        png = driver.get_screenshot_as_png(); im = Image.open(io.BytesIO(png)).convert("RGB")
        if im.width > 1280 * scale: im = im.resize((1280 * scale, int(im.height * 1280 * scale / im.width)), Image.LANCZOS)
        os.makedirs(args.out, exist_ok=True)
        path = os.path.join(args.out, name + ".webp"); im.save(path, "WEBP", quality=82, method=6); print(name, im.size, os.path.getsize(path) // 1024, "Ko")
    js = driver.execute_script
    driver.get(f"http://127.0.0.1:{port}"); js("localStorage.setItem('mcmanager-lang','fr')"); driver.get(f"http://127.0.0.1:{port}"); time.sleep(2)
    hide_alerts = lambda: js("document.getElementById('alert-list')?.remove()")  # alerte disque propre au poste de capture
    js("Appearance.set({theme:'graphite', accent:'#6d7cff', bg:'dots'})")
    js("document.querySelector('.nav-item[data-view=\"dashboard\"]').click()"); time.sleep(1.5)
    hide_alerts(); shot("shot-dashboard", 1280, 820)
    # Sélection du serveur Bedrock par le vrai sélecteur de l'interface, puis « Packs & mondes ».
    js(f"const sel=document.getElementById('server-select'); sel.value='{bed_id}'; sel.dispatchEvent(new Event('change',{{bubbles:true}}));"); time.sleep(1)
    js("document.querySelector('.nav-item[data-view=\"addons\"]').click()"); time.sleep(2)
    shot("shot-bedrock", 1280, 900)
    js("document.querySelector('.nav-item[data-view=\"discord\"]').click()"); time.sleep(2)
    shot("shot-discord", 1280, 900)
    js("Appearance.set({theme:'custom'}); Appearance.deriveFrom('#14101f','#ece6ff'); Appearance.set({accent:'#a78bfa'})")
    js("document.querySelector('.nav-item[data-view=\"dashboard\"]').click()"); time.sleep(1.5)
    hide_alerts(); shot("shot-theme", 1280, 820)
    js("Appearance.set({theme:'graphite', accent:'#6d7cff'})")
    driver.set_window_size(390, 800); time.sleep(0.8)
    js("document.querySelector('.nav-item[data-view=\"dashboard\"]').click()"); time.sleep(1.2)
    js("document.getElementById('menu-toggle').click()"); time.sleep(0.6)
    shot("shot-mobile", 390, 800, scale=2)
finally:
    if driver: driver.quit()
    proc.terminate()
    try: proc.wait(10)
    except Exception: proc.kill()
    shutil.rmtree(data, ignore_errors=True)
