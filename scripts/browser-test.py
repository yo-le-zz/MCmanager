#!/usr/bin/env python3
"""Test de l'interface dans un vrai navigateur (Firefox headless + Selenium) contre le vrai binaire.

Usage :
  python3 scripts/browser-test.py --binary target/debug/mcmanager --firefox /chemin/firefox --geckodriver /chemin/geckodriver [--shots dossier]

Vérifie : aucune erreur JavaScript, navigation dans toutes les vues, thème personnalisé (couleurs réellement
appliquées), fond animé (images tracées, temps de dessin), vues Assistant et Discord (nouveaux champs), détection Java.
Sort en code 1 au premier écart. Nécessite : pip install selenium.
"""
import argparse, base64, hashlib, hmac, json, os, shutil, socket, struct, subprocess, sys, tempfile, time, urllib.request

from selenium import webdriver
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service
from selenium.webdriver.common.by import By

ap = argparse.ArgumentParser()
ap.add_argument("--binary", required=True); ap.add_argument("--firefox", required=True); ap.add_argument("--geckodriver", required=True)
ap.add_argument("--web", default=os.path.join(os.path.dirname(__file__), "..", "web")); ap.add_argument("--shots", default="")
args = ap.parse_args()

def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0)); return s.getsockname()[1]

port, data = free_port(), tempfile.mkdtemp(prefix="mcm-browser-")
env = dict(os.environ, MCMANAGER_PORT=str(port), MCMANAGER_HOST="127.0.0.1", MCMANAGER_DATA_DIR=data, MCMANAGER_NO_BROWSER="1", MCMANAGER_WEB_DIR=os.path.abspath(args.web))
server = subprocess.Popen([args.binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
base = f"http://127.0.0.1:{port}"
failures = []
def check(ok, msg):
    print(("ok   - " if ok else "FAIL - ") + msg)
    if not ok: failures.append(msg)

driver = None
try:
    for _ in range(100):
        try: urllib.request.urlopen(base + "/", timeout=1); break
        except Exception: time.sleep(0.2)
    else: raise SystemExit("le serveur n'a pas démarré")

    opts = Options(); opts.add_argument("-headless"); opts.binary_location = args.firefox
    driver = webdriver.Firefox(options=opts, service=Service(args.geckodriver))
    driver.set_window_size(1280, 900); driver.set_script_timeout(30)
    shot = lambda n: args.shots and (os.makedirs(args.shots, exist_ok=True), driver.save_screenshot(os.path.join(args.shots, n + ".png")))
    js = lambda code, *a: driver.execute_script(code, *a)
    errors = lambda: js("return window.__mcmErrors || []")

    driver.get(base); time.sleep(1.5)
    check(js("return !!window.Appearance && !!window.BackgroundFX"), "ui.js chargé (Appearance, BackgroundFX)")
    check(errors() == [], f"aucune erreur JS au chargement {errors()}")
    shot("01-accueil")

    # Navigation : chaque vue sans serveur doit se rendre sans erreur.
    views = js("return Array.from(document.querySelectorAll('.nav-item[data-view]')).filter(b=>!b.dataset.needsServer).map(b=>b.dataset.view)")
    for v in views:
        js("document.querySelector(`.nav-item[data-view=\"${arguments[0]}\"]`).click()", v); time.sleep(0.6)
        check(errors() == [], f"vue « {v} » sans erreur JS {errors()}")
    check(len(views) >= 5, f"{len(views)} vues explorées")

    # Réglages : Détecter Java et sections Discord.
    js("document.querySelector('.nav-item[data-view=\"settings\"]').click()"); time.sleep(1)
    check(len(driver.find_elements(By.ID, "st-java-detect")) == 1, "bouton « Détecter » Java présent dans les réglages")
    driver.find_element(By.ID, "st-java-detect").click(); time.sleep(2.5)
    found = driver.find_element(By.ID, "st-java-found").text
    check(found != "" and "Recherche" not in found, f"détection Java répond ({found[:60]!r})")
    for el in ("dc-presence", "dc-activity-type", "dc-activity-text", "dc-presence-apply", "dc-join-host"):
        check(len(driver.find_elements(By.ID, el)) == 1, f"champ Discord #{el}")
    shot("02-reglages-discord")

    # Statut du bot : l'API répond et l'interface l'enregistre.
    driver.execute_script("document.getElementById('dc-presence').value='dnd';document.getElementById('dc-activity-type').value='watching';document.getElementById('dc-activity-text').value='{online}/{servers} serveurs'")
    driver.find_element(By.ID, "dc-presence-apply").click(); time.sleep(1.2)
    cfg = json.load(urllib.request.urlopen(base + "/api/discord/config"))
    check((cfg["bot_presence"], cfg["activity_type"], cfg["activity_text"]) == ("dnd", "watching", "{online}/{servers} serveurs"), "statut du bot enregistré via l'interface")
    check(errors() == [], f"aucune erreur JS dans les réglages {errors()}")

    # Thème personnalisé : les couleurs sont réellement appliquées à la page.
    js("Appearance.set({theme:'custom'}); Appearance.setColor('bg','#102030'); Appearance.setColor('text','#f0e0d0')"); time.sleep(0.4)
    bg = js("return getComputedStyle(document.body).backgroundColor")
    check(bg.replace(" ", "") == "rgb(16,32,48)", f"fond personnalisé appliqué ({bg})")
    shot("03-theme-personnalise")
    imp = js("return Appearance.importJson(JSON.stringify({mcmanagerTheme:1,colors:{bg:'url(//evil.test)'},css:'a{background:url(//evil.test/x)}'}))")
    check(imp.get("ok") is True and "evil.test" not in js("return (document.getElementById('mcm-custom-css')||{textContent:''}).textContent"), "import hostile neutralisé dans le navigateur")
    js("Appearance.set({theme:'graphite'})")

    # Fond animé : on promène la souris et on mesure le temps de dessin.
    js("Appearance.set({bg:'dots', bgInteractive:true}); window.__mcmPerf.frames=0; window.__mcmPerf.totalMs=0; window.__mcmPerf.maxMs=0"); time.sleep(0.5)
    t = driver.execute_async_script("""
      const done = arguments[arguments.length-1]; let i = 0;
      const step = () => { window.dispatchEvent(new PointerEvent('pointermove',{clientX:100+i*9,clientY:200+Math.sin(i/5)*120})); if (++i < 90) requestAnimationFrame(step); else setTimeout(()=>done(BackgroundFX.stats), 200); };
      requestAnimationFrame(step);""")
    check(t["frames"] >= 20, f"le fond a tracé {t['frames']} images pendant le mouvement")
    check(t["avgMs"] < 9, f"temps de dessin moyen {t['avgMs']:.2f} ms (< 9 ms) ; max {t['maxMs']:.1f} ms ; points {t['points']} ; qualité {t['quality']}")
    shot("04-fond-anime")
    js("Appearance.set({bg:'world'})"); time.sleep(1.2)
    check(js("return BackgroundFX.stats.points") > 500, f"carte du monde chargée ({js('return BackgroundFX.stats.points')} points)")
    js("Appearance.set({bg:'dots'})")

    # Assistant : la vue se rend, les nouveaux éléments existent.
    js("document.querySelector('.nav-item[data-view=\"assistant\"]').click()"); time.sleep(1)
    check(len(driver.find_elements(By.ID, "ai-chat-stop")) == 1 and len(driver.find_elements(By.ID, "ai-tools-mode")) == 1, "vue Assistant : bouton Arrêter et mode des outils")
    shot("05-assistant")
    check(errors() == [], f"aucune erreur JS finale {errors()}")

    # --- Menu mobile : écran de téléphone, tiroir fermé puis ouvert ---
    driver.set_window_size(390, 800); time.sleep(0.6)
    js("window.scrollTo(0,0)")
    vis = lambda: js("const r=document.getElementById('sidebar').getBoundingClientRect(); return [r.left, r.right]")
    check(vis()[1] <= 1, f"mobile : barre latérale repliée hors écran {vis()}")
    check(js("return getComputedStyle(document.getElementById('menu-toggle')).display") != "none", "mobile : bouton menu visible")
    driver.find_element(By.ID, "menu-toggle").click(); time.sleep(0.5)
    check(vis()[0] >= -1 and js("return document.body.classList.contains('menu-open')"), f"mobile : tiroir ouvert {vis()}")
    shot("06-mobile-menu")
    js("document.querySelector('.nav-item[data-view=\"dashboard\"]').click()"); time.sleep(0.6)
    check(not js("return document.body.classList.contains('menu-open')"), "mobile : le tiroir se referme après le choix d'une vue")
    check(js("return document.documentElement.scrollWidth <= window.innerWidth + 1"), "mobile : pas de défilement horizontal de la page")
    check(errors() == [], f"aucune erreur JS en mode mobile {errors()}")
    driver.set_window_size(1280, 900)
finally:
    if driver: driver.quit()
    server.terminate()
    try: server.wait(5)
    except Exception: server.kill()
    shutil.rmtree(data, ignore_errors=True)

def totp(secret_b32, t=None):
    key = base64.b32decode(secret_b32.replace(" ", "").upper() + "=" * (-len(secret_b32.replace(" ", "")) % 8))
    c = int((t or time.time()) // 30)
    h = hmac.new(key, struct.pack(">Q", c), hashlib.sha1).digest()
    o = h[19] & 15
    return f"{(struct.unpack('>I', h[o:o+4])[0] & 0x7fffffff) % 1000000:06d}"

# --- Phase 2 : serveur avec comptes, double authentification de bout en bout dans le navigateur ---
port2, data2 = free_port(), tempfile.mkdtemp(prefix="mcm-browser2-")
env2 = dict(env, MCMANAGER_PORT=str(port2), MCMANAGER_DATA_DIR=data2, MCMANAGER_AUTH="1", MCMANAGER_ADMIN_USER="admin", MCMANAGER_ADMIN_PASSWORD="motdepasse-test-1")
server2 = subprocess.Popen([args.binary], env=env2, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
driver2 = None
try:
    base2 = f"http://127.0.0.1:{port2}"
    for _ in range(100):
        try: urllib.request.urlopen(base2 + "/healthz", timeout=1); break
        except Exception: time.sleep(0.2)
    opts2 = Options(); opts2.add_argument("-headless"); opts2.binary_location = args.firefox
    driver2 = webdriver.Firefox(options=opts2, service=Service(args.geckodriver)); driver2.set_window_size(1280, 900)
    wait = lambda f, n=40: next((r for _ in range(n) if (r := f()) or time.sleep(0.25)), None)
    def login(code=None):
        driver2.get(base2); wait(lambda: driver2.find_elements(By.ID, "lg-user"))
        driver2.find_element(By.ID, "lg-user").send_keys("admin"); driver2.find_element(By.ID, "lg-pass").send_keys("motdepasse-test-1")
        if code:
            driver2.find_element(By.ID, "lg-code").send_keys(code)
        driver2.find_element(By.ID, "lg-submit").click()
        # La dérivation PBKDF2 est lente en build debug : on attend le résultat (page rechargée, ou erreur affichée).
        wait(lambda: not driver2.find_elements(By.ID, "lg-user") or driver2.find_element(By.ID, "lg-error").text, 80)
    login()
    check(len(driver2.find_elements(By.ID, "content")) == 1 and not driver2.find_elements(By.ID, "lg-user"), "phase 2 : connexion par mot de passe")
    driver2.execute_script("document.querySelector('.nav-item[data-view=\"users\"]').click()"); time.sleep(1.2)
    check(len(driver2.find_elements(By.ID, "totp-start")) == 1, "carte Double authentification présente")
    driver2.find_element(By.ID, "totp-start").click(); time.sleep(1)
    secret = driver2.find_element(By.CSS_SELECTOR, "#totp-setup-area code").text
    check(len(secret.replace(" ", "")) == 32, f"clé de configuration affichée ({len(secret.replace(' ', ''))} caractères)")
    driver2.find_element(By.ID, "totp-code").send_keys(totp(secret)); driver2.find_element(By.ID, "totp-confirm").click(); time.sleep(1.2)
    codes = driver2.find_element(By.CSS_SELECTOR, "#totp-setup-area pre").text.split()
    check(len(codes) == 8, f"{len(codes)} codes de secours affichés")
    if args.shots: os.makedirs(args.shots, exist_ok=True); driver2.save_screenshot(os.path.join(args.shots, "07-totp-codes.png"))
    driver2.find_element(By.ID, "totp-done").click(); time.sleep(1)
    # Déconnexion puis reconnexion : le mot de passe seul ne suffit plus.
    driver2.execute_script("fetch('/auth/logout',{method:'POST'})"); time.sleep(0.5)
    login()
    wait(lambda: driver2.find_element(By.ID, "lg-code-row").is_displayed(), 20)
    check(driver2.find_element(By.ID, "lg-code-row").is_displayed(), "le champ de code apparaît après le mot de passe")
    check(len(driver2.find_elements(By.ID, "lg-user")) == 1, "toujours sur l'écran de connexion sans code")
    driver2.find_element(By.ID, "lg-code").send_keys("000000"); driver2.find_element(By.ID, "lg-submit").click()
    err = wait(lambda: driver2.find_element(By.ID, "lg-error").text, 60) or ""   # la dérivation du mot de passe est lente en build debug
    check("incorrect" in err.lower(), f"mauvais code refusé ({err!r})")
    driver2.find_element(By.ID, "lg-code").clear(); driver2.find_element(By.ID, "lg-code").send_keys(codes[0]); driver2.find_element(By.ID, "lg-submit").click()
    wait(lambda: not driver2.find_elements(By.ID, "lg-user"), 80)
    check(not driver2.find_elements(By.ID, "lg-user"), "connexion réussie avec un code de secours")
finally:
    if driver2: driver2.quit()
    server2.terminate()
    try: server2.wait(5)
    except Exception: server2.kill()
    shutil.rmtree(data2, ignore_errors=True)

print(f"\n{'ÉCHEC : ' + str(len(failures)) + ' vérification(s)' if failures else 'Tous les tests navigateur passent'}")
sys.exit(1 if failures else 0)
