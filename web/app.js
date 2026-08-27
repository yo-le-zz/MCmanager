// MCManager — frontend SPA (no build step, plain JS on purpose so it can be
// served as static files from the Rust binary on every platform).

const state = {
  view: "dashboard",
  servers: [],
  currentServerId: null,
  ws: null,
  currentPath: "",
};

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

function toast(msg, kind = "info") {
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.textContent = msg;
  $("#toast-container").appendChild(el);
  setTimeout(() => el.remove(), 4500);
}

async function api(path, opts = {}) {
  const res = await fetch(`/api${path}`, {
    headers: opts.body ? { "Content-Type": "application/json" } : {},
    ...opts,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    const msg = data.error || `Erreur ${res.status}`;
    toast(msg, "error");
    throw new Error(msg);
  }
  return data;
}

function currentServer() {
  return state.servers.find((s) => s.id === state.currentServerId);
}

// ───────────────────────── navigation ─────────────────────────

function setupNav() {
  $$(".nav-item").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.needsServer && !state.currentServerId) {
        toast("Sélectionnez d'abord un serveur.", "error");
        return;
      }
      state.view = btn.dataset.view;
      $$(".nav-item").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      render();
    });
  });

  $("#server-select").addEventListener("change", (e) => {
    state.currentServerId = e.target.value || null;
    render();
  });
}

async function refreshServerList(selectId) {
  state.servers = await api("/servers");
  const sel = $("#server-select");
  sel.innerHTML = `<option value="">${t("nav.none")}</option>` +
    state.servers.map((s) => `<option value="${s.id}">${escapeHtml(s.name)} (${s.mc_version})</option>`).join("");
  if (selectId) {
    state.currentServerId = selectId;
  }
  if (state.currentServerId && state.servers.some((s) => s.id === state.currentServerId)) {
    sel.value = state.currentServerId;
  } else {
    state.currentServerId = null;
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

// ───────────────────────── couleurs console (ANSI + codes §) ─────────────────────────

const ANSI_COLOR_MAP = {
  30: "#45475a", 31: "#f38ba8", 32: "#a6e3a1", 33: "#f9e2af", 34: "#89b4fa", 35: "#f5c2e7", 36: "#94e2d5", 37: "#bac2de",
  90: "#585b70", 91: "#f38ba8", 92: "#a6e3a1", 93: "#f9e2af", 94: "#89b4fa", 95: "#f5c2e7", 96: "#94e2d5", 97: "#a6adc8",
};
const MC_COLOR_MAP = {
  "0": "#000000", "1": "#0000aa", "2": "#00aa00", "3": "#00aaaa", "4": "#aa0000", "5": "#aa00aa", "6": "#ffaa00", "7": "#aaaaaa",
  "8": "#555555", "9": "#5555ff", a: "#55ff55", b: "#55ffff", c: "#ff5555", d: "#ff55ff", e: "#ffff55", f: "#ffffff",
};

/// Converts one raw console line to safe, colorized HTML: escapes the text
/// first (so nothing in server/plugin output can inject markup), then
/// turns ANSI SGR sequences (`\x1b[NNm`, what Paper/Spigot/log4j2 print
/// when color is enabled) and literal Minecraft `§x` codes into `<span>`s.
/// Unrecognized/unsupported codes are just dropped rather than shown as
/// garbage control characters.
function consoleLineToHtml(line) {
  let html = escapeHtml(line);

  // ANSI: \x1b[<codes>m - split on the escape sequences, track open spans.
  const ansiRe = /\x1b\[([0-9;]*)m/g;
  let openSpans = 0;
  html = html.replace(ansiRe, (_, codes) => {
    const parts = codes.split(";").filter(Boolean).map(Number);
    if (parts.length === 0 || parts.includes(0)) {
      const closing = "</span>".repeat(openSpans);
      openSpans = 0;
      return closing;
    }
    let style = "";
    for (const code of parts) {
      if (ANSI_COLOR_MAP[code]) style += `color:${ANSI_COLOR_MAP[code]};`;
      if (code === 1) style += "font-weight:700;";
      if (code === 4) style += "text-decoration:underline;";
      if (code === 3) style += "font-style:italic;";
    }
    if (!style) return "";
    openSpans++;
    return `<span style="${style}">`;
  });
  html += "</span>".repeat(openSpans);

  // Minecraft §-codes (appear literally when a plugin logs colored chat
  // without ANSI translation). escapeHtml doesn't touch §, so this still
  // matches post-escape.
  const mcRe = /§([0-9a-fk-or])/gi;
  let mcOpen = 0;
  html = html.replace(mcRe, (_, code) => {
    const c = code.toLowerCase();
    if (c === "r") {
      const closing = "</span>".repeat(mcOpen);
      mcOpen = 0;
      return closing;
    }
    if (MC_COLOR_MAP[c]) {
      mcOpen++;
      return `<span style="color:${MC_COLOR_MAP[c]}">`;
    }
    return ""; // formatting codes (k,l,m,n,o) other than color: drop, not worth the complexity
  });
  html += "</span>".repeat(mcOpen);

  return html;
}

// ───────────────────────── apparence de la console (police, taille) ─────────────────────────

const CONSOLE_APPEARANCE_DEFAULTS = { fontSize: 12.5, fontFamily: "Consolas, 'Courier New', monospace" };

function getConsoleAppearance() {
  try {
    return { ...CONSOLE_APPEARANCE_DEFAULTS, ...JSON.parse(localStorage.getItem("mcmanager-console-appearance") || "{}") };
  } catch {
    return { ...CONSOLE_APPEARANCE_DEFAULTS };
  }
}

function setConsoleAppearance(partial) {
  const merged = { ...getConsoleAppearance(), ...partial };
  localStorage.setItem("mcmanager-console-appearance", JSON.stringify(merged));
  applyConsoleAppearance();
}

function applyConsoleAppearance() {
  const a = getConsoleAppearance();
  document.documentElement.style.setProperty("--console-font-size", `${a.fontSize}px`);
  document.documentElement.style.setProperty("--console-font-family", a.fontFamily);
}

// ───────────────────────── render dispatcher ─────────────────────────

async function render() {
  const content = $("#content");
  content.innerHTML = '<div class="empty-state">Chargement…</div>';
  try {
    switch (state.view) {
      case "dashboard": return renderDashboard();
      case "servers": return renderServers();
      case "console": return renderConsole();
      case "files": return renderFiles();
      case "addons": return renderAddons();
      case "marketplace": return renderMarketplace();
      case "schematics": return renderSchematics();
      case "whitelist": return renderWhitelist();
      case "properties": return renderProperties();
      case "backups": return renderBackups();
      case "stats": return renderStats();
      case "network": return renderNetwork();
      case "remote": return renderRemote();
      case "docs": return renderDocs();
      case "assistant": return renderAssistant();
      case "settings": return renderSettings();
      default: content.innerHTML = "";
    }
  } catch (e) {
    console.error(e);
  }
}

// ───────────────────────── dashboard ─────────────────────────

async function renderDashboard() {
  const content = $("#content");
  await refreshServerList();
  const rows = await Promise.all(state.servers.map(async (s) => {
    try {
      const status = await api(`/servers/${s.id}/status`);
      return { s, status };
    } catch {
      return { s, status: null };
    }
  }));

  content.innerHTML = `
    <h1>${t('view.dashboard')}</h1>
    <div class="subtitle">Vue d'ensemble de tous vos serveurs Minecraft.</div>
    <div class="grid">
      <div class="stat-card"><div class="label">Serveurs</div><div class="value">${state.servers.length}</div></div>
      <div class="stat-card"><div class="label">En ligne</div><div class="value">${rows.filter(r => r.status?.running).length}</div></div>
      <div class="stat-card"><div class="label">Joueurs connectés</div><div class="value">${rows.reduce((a, r) => a + (r.status?.players_online || 0), 0)}</div></div>
    </div>
    <div class="card">
      <h2>Serveurs</h2>
      ${state.servers.length === 0 ? '<div class="empty-state">Aucun serveur pour le moment. Allez dans <b>Serveurs</b> pour en créer un.</div>' : `
      <table>
        <thead><tr><th>Nom</th><th>Type</th><th>Version</th><th>Statut</th><th>Joueurs</th><th>CPU</th><th>RAM</th><th></th></tr></thead>
        <tbody>
          ${rows.map(({ s, status }) => `
            <tr>
              <td><b>${escapeHtml(s.name)}</b></td>
              <td>${s.loader}</td>
              <td>${s.mc_version}</td>
              <td>${status?.running ? '<span class="badge badge-green">En ligne</span>' : '<span class="badge badge-red">Arrêté</span>'}</td>
              <td>${status?.players_online != null ? `${status.players_online}/${status.players_max}` : '—'}</td>
              <td>${status ? status.cpu_percent.toFixed(1) + '%' : '—'}</td>
              <td>${status ? status.mem_mb.toFixed(0) + ' Mo' : '—'}</td>
              <td>
                ${status?.running
                  ? `<button class="btn-red" data-stop="${s.id}">Stop</button>`
                  : `<button class="btn-green" data-start="${s.id}">Start</button>`}
              </td>
            </tr>`).join("")}
        </tbody>
      </table>`}
    </div>
  `;

  $$("[data-start]").forEach((b) => b.addEventListener("click", async () => {
    await api(`/servers/${b.dataset.start}/start`, { method: "POST" });
    toast("Démarrage du serveur…", "success");
    renderDashboard();
  }));
  $$("[data-stop]").forEach((b) => b.addEventListener("click", async () => {
    if (!confirm("Arrêter ce serveur ? Les joueurs connectés seront déconnectés.")) return;
    await api(`/servers/${b.dataset.stop}/stop`, { method: "POST" });
    toast("Arrêt demandé.", "success");
    renderDashboard();
  }));
}

// ───────────────────────── servers (list + create wizard) ─────────────────────────

async function renderServers() {
  const content = $("#content");
  await refreshServerList();
  content.innerHTML = `
    <h1>${t('view.servers')}</h1>
    <div class="subtitle">Créez et gérez vos serveurs Minecraft.</div>
    <div class="toolbar">
      <button class="btn-blue" id="new-server-btn">+ Nouveau serveur</button>
      <button class="btn-ghost" id="import-server-btn">📂 Importer un serveur existant</button>
    </div>
    <div id="server-list" class="grid"></div>
    <div id="wizard" class="card hidden"></div>
    <div id="import-form" class="card hidden"></div>
  `;
  const list = $("#server-list");
  if (state.servers.length === 0) {
    list.innerHTML = '<div class="empty-state">Aucun serveur. Cliquez sur "Nouveau serveur" pour commencer — MCManager télécharge et configure tout automatiquement.</div>';
  } else {
    list.innerHTML = state.servers.map((s) => `
      <div class="stat-card">
        <div class="label">${s.loader.toUpperCase()} · ${s.mc_version}</div>
        <div class="value" style="font-size:16px">${escapeHtml(s.name)}</div>
        <div style="margin-top:10px;display:flex;gap:6px;flex-wrap:wrap">
          <button class="btn-ghost" data-open="${s.id}">Ouvrir</button>
          <button class="btn-ghost" data-explorer="${s.id}">🗂 ${t('files.openExplorer')}</button>
          <button class="btn-red" data-del="${s.id}">${t('common.delete')}</button>
        </div>
      </div>
    `).join("");
    $$("[data-open]").forEach((b) => b.addEventListener("click", () => {
      state.currentServerId = b.dataset.open;
      $("#server-select").value = state.currentServerId;
      state.view = "console";
      $$(".nav-item").forEach((n) => n.classList.toggle("active", n.dataset.view === "console"));
      render();
    }));
    $$("[data-explorer]").forEach((b) => b.addEventListener("click", async () => {
      try {
        await api(`/servers/${b.dataset.explorer}/open-folder`, { method: "POST" });
      } catch {
        toast(t('files.explorerFailed'), "error");
      }
    }));
    $$("[data-del]").forEach((b) => b.addEventListener("click", async () => {
      if (!confirm("Supprimer définitivement ce serveur et tous ses fichiers (y compris les sauvegardes) ?")) return;
      const result = await api(`/servers/${b.dataset.del}`, { method: "DELETE" });
      if (result.warnings && result.warnings.length) {
        toast("Serveur retiré, mais : " + result.warnings.join(" / "), "error");
      } else {
        toast("Serveur supprimé (dossiers et sauvegardes inclus).", "success");
      }
      renderServers();
    }));
  }

  $("#new-server-btn").addEventListener("click", () => {
    $("#import-form").classList.add("hidden");
    openWizard();
  });
  $("#import-server-btn").addEventListener("click", () => {
    $("#wizard").classList.add("hidden");
    openImportForm();
  });
}

async function openImportForm() {
  const form = $("#import-form");
  form.classList.remove("hidden");
  form.innerHTML = `
    <h2>Importer un serveur existant</h2>
    <div class="subtitle">Pour un serveur déjà présent sur cette machine (dossier avec un .jar dedans).</div>
    <div class="form-grid">
      <div class="form-row"><label>Nom</label><input id="i-name" placeholder="Mon serveur importé"></div>
      <div class="form-row">
        <label>Type de serveur</label>
        <select id="i-loader">
          <option value="vanilla">Vanilla</option>
          <option value="paper" selected>Paper</option>
          <option value="purpur">Purpur</option>
          <option value="spigot">Spigot</option>
          <option value="fabric">Fabric</option>
          <option value="quilt">Quilt</option>
          <option value="forge">Forge</option>
          <option value="neoforge">NeoForge</option>
        </select>
      </div>
      <div class="form-row"><label>Version de Minecraft</label><input id="i-version" placeholder="1.21.11"></div>
      <div class="form-row"><label>Dossier du serveur (chemin absolu sur cette machine)</label><input id="i-path" placeholder="/home/moi/serveurs/survie"></div>
      <div class="form-row"><label>Nom du .jar (optionnel, auto-détecté sinon)</label><input id="i-jar" placeholder="server.jar"></div>
      <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input id="i-autorestart" type="checkbox" style="width:auto"> Redémarrage automatique en cas de crash</label></div>
    </div>
    <button class="btn-green" id="i-submit">Importer</button>
    <span id="i-status" style="margin-left:12px;color:var(--subtext1)"></span>
  `;
  $("#i-submit").addEventListener("click", async () => {
    const status = $("#i-status");
    $("#i-submit").disabled = true;
    try {
      const body = {
        name: $("#i-name").value || "Serveur importé",
        loader: $("#i-loader").value,
        mc_version: $("#i-version").value,
        folder_path: $("#i-path").value,
        jar_name: $("#i-jar").value || null,
        auto_restart: $("#i-autorestart").checked,
      };
      const server = await api("/servers/import", { method: "POST", body: JSON.stringify(body) });
      toast(`Serveur "${server.name}" importé !`, "success");
      renderServers();
    } catch (e) {
      status.textContent = "Échec : " + e.message;
    } finally {
      $("#i-submit").disabled = false;
    }
  });
}

async function openWizard() {
  const wizard = $("#wizard");
  wizard.classList.remove("hidden");
  wizard.innerHTML = `
    <h2>Nouveau serveur</h2>
    <div class="form-row">
      <label>Nom du serveur</label>
      <input id="w-name" placeholder="Mon super serveur">
    </div>
    <div class="form-grid">
      <div class="form-row">
        <label>Type de serveur</label>
        <select id="w-loader">
          <option value="vanilla">Vanilla (officiel Mojang)</option>
          <option value="paper" selected>Paper (plugins, performant — recommandé)</option>
          <option value="purpur">Purpur (Paper + options avancées)</option>
          <option value="spigot">Spigot</option>
          <option value="fabric">Fabric (mods, léger)</option>
          <option value="quilt">Quilt (mods, fork moderne de Fabric)</option>
          <option value="forge">Forge (mods)</option>
          <option value="neoforge">NeoForge (mods, successeur de Forge)</option>
        </select>
      </div>
      <div class="form-row">
        <label>Version de Minecraft</label>
        <select id="w-version"><option>Chargement…</option></select>
      </div>
      <div class="form-row" id="w-build-row">
        <label>Build / version du loader (optionnel)</label>
        <select id="w-build"><option value="">Dernière stable (recommandé)</option></select>
      </div>
      <div class="form-row">
        <label>RAM minimum (Mo)</label>
        <input id="w-xms" type="number" value="1024">
      </div>
      <div class="form-row">
        <label>RAM maximum (Mo)</label>
        <input id="w-xmx" type="number" value="2048">
      </div>
      <div class="form-row">
        <label>Port</label>
        <input id="w-port" type="number" value="25565">
      </div>
      <div class="form-row">
        <label style="display:flex;align-items:center;gap:8px;margin-top:20px">
          <input id="w-eula" type="checkbox" style="width:auto"> J'accepte l'EULA Minecraft (mojang.com/eula)
        </label>
      </div>
    </div>
    <details style="margin:6px 0 16px">
      <summary style="cursor:pointer;color:var(--subtext1);font-size:13px">Options avancées</summary>
      <div class="form-grid" style="margin-top:12px">
        <div class="form-row">
          <label style="display:flex;align-items:center;gap:8px">
            <input id="w-aikar" type="checkbox" style="width:auto"> Flags de performance (Aikar) — recommandé pour Paper/Purpur/Spigot
          </label>
        </div>
        <div class="form-row">
          <label style="display:flex;align-items:center;gap:8px">
            <input id="w-autorestart" type="checkbox" style="width:auto"> Redémarrage automatique en cas de crash
          </label>
        </div>
        <div class="form-row">
          <label>Sauvegarde automatique (minutes, vide = désactivé)</label>
          <input id="w-autobackup" type="number" placeholder="ex: 60">
        </div>
        <div class="form-row">
          <label>Arguments JVM additionnels (séparés par des espaces)</label>
          <input id="w-extraargs" placeholder="ex: -Dfile.encoding=UTF-8">
        </div>
      </div>
    </details>
    <button class="btn-green" id="w-create">Créer le serveur</button>
    <span id="w-status" style="margin-left:12px;color:var(--subtext1)"></span>
  `;

  const loaderSel = $("#w-loader");
  const versionSel = $("#w-version");
  const buildSel = $("#w-build");
  const buildRow = $("#w-build-row");

  const LOADERS_WITH_BUILDS = ["paper", "purpur", "fabric", "quilt", "forge"];

  async function loadBuilds() {
    if (!LOADERS_WITH_BUILDS.includes(loaderSel.value) || !versionSel.value) {
      buildRow.classList.add("hidden");
      return;
    }
    buildRow.classList.remove("hidden");
    buildSel.innerHTML = '<option value="">Dernière stable (recommandé)</option>';
    try {
      const builds = await api(`/loaders/${loaderSel.value}/builds?version=${encodeURIComponent(versionSel.value)}`);
      buildSel.innerHTML += builds.map((b) => `<option value="${escapeHtml(b.value)}">${escapeHtml(b.label)}</option>`).join("");
    } catch { /* garde juste l'option par défaut */ }
  }

  async function loadVersions() {
    versionSel.innerHTML = "<option>Chargement…</option>";
    const versions = await api(`/loaders/${loaderSel.value}/versions`);
    versionSel.innerHTML = versions.map((v) => `<option value="${v}">${v}</option>`).join("");
    await loadBuilds();
  }
  loaderSel.addEventListener("change", loadVersions);
  versionSel.addEventListener("change", loadBuilds);
  await loadVersions();

  $("#w-create").addEventListener("click", async () => {
    if (!$("#w-eula").checked) {
      toast("Vous devez accepter l'EULA Minecraft pour continuer.", "error");
      return;
    }
    const status = $("#w-status");
    status.textContent = "Téléchargement et configuration en cours (cela peut prendre une minute)…";
    $("#w-create").disabled = true;
    try {
      const extraArgs = $("#w-extraargs").value.trim();
      const autoBackup = $("#w-autobackup").value.trim();
      const body = {
        name: $("#w-name").value || "Nouveau serveur",
        loader: loaderSel.value,
        mc_version: versionSel.value,
        loader_version: buildSel.value || null,
        xms_mb: parseInt($("#w-xms").value, 10),
        xmx_mb: parseInt($("#w-xmx").value, 10),
        port: parseInt($("#w-port").value, 10),
        eula_accepted: true,
        aikar_flags: $("#w-aikar").checked,
        auto_restart: $("#w-autorestart").checked,
        extra_args: extraArgs ? extraArgs.split(/\s+/) : [],
        auto_backup_minutes: autoBackup ? parseInt(autoBackup, 10) : null,
      };
      const server = await api("/servers", { method: "POST", body: JSON.stringify(body) });
      toast(`Serveur "${server.name}" créé !`, "success");
      state.currentServerId = server.id;
      renderServers();
    } catch (e) {
      status.textContent = "Échec : " + e.message;
    } finally {
      $("#w-create").disabled = false;
    }
  });
}

// ───────────────────────── console ─────────────────────────

function renderConsole() {
  const s = currentServer();
  const content = $("#content");
  content.innerHTML = `
    <h1>${t('view.console')} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">${s.loader} · ${s.mc_version} · port ${s.port}</div>
    <div class="toolbar">
      <button class="btn-green" id="c-start">▶ Démarrer</button>
      <button class="btn-yellow" id="c-restart">⟳ Redémarrer</button>
      <button class="btn-red" id="c-stop">⏹ Arrêter</button>
      <button class="btn-ghost" id="c-kill">✕ Forcer l'arrêt</button>
      <button class="btn-ghost" id="c-clear">🧹 Effacer la console</button>
      <button class="btn-ghost" id="c-debug">🩺 Diagnostiquer un crash</button>
    </div>
    <div class="console" id="console-out"></div>
    <div class="console-input-row console-input-row-multiline">
      <textarea id="console-cmd" rows="3" placeholder="Une commande par ligne (ex: give Steve diamond_sword{...} 1). Entrée = nouvelle ligne, Ctrl+Entrée ou le bouton = tout exécuter."></textarea>
      <div class="console-input-actions">
        <button class="btn-blue" id="console-send">▶ Exécuter</button>
        <span class="meta" id="console-line-count"></span>
      </div>
    </div>
  `;
  applyConsoleAppearance();

  const out = $("#console-out");
  function appendLine(line) {
    const atBottom = out.scrollTop + out.clientHeight >= out.scrollHeight - 30;
    const row = document.createElement("div");
    row.innerHTML = consoleLineToHtml(line);
    out.appendChild(row);
    if (atBottom) out.scrollTop = out.scrollHeight;
  }

  if (state.ws) { state.ws.close(); state.ws = null; }
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${proto}://${location.host}/api/servers/${s.id}/ws`);
  ws.onmessage = (ev) => appendLine(ev.data);
  ws.onerror = () => {};
  state.ws = ws;

  // Commands that stop/restart the server are dangerous to send by accident
  // (fat-fingering "stop" while trying to type a chat message loses the
  // whole session) - confirm before actually sending them.
  const DANGEROUS_COMMANDS = ["stop", "end", "shutdown", "restart", "reload", "reload confirm"];

  async function sendOne(cmd) {
    if (DANGEROUS_COMMANDS.includes(cmd.toLowerCase())) {
      if (!confirm(`Cette commande ("${cmd}") va arrêter ou recharger le serveur. Confirmer ?`)) return false;
    }
    if (ws.readyState === 1) ws.send(cmd);
    else await api(`/servers/${s.id}/command`, { method: "POST", body: JSON.stringify({ cmd }) });
    return true;
  }

  // Runs every non-empty line as its own command, in order. A small delay
  // between each keeps them in the right sequence on the server side
  // (console commands are processed one at a time; firing them all at once
  // with no gap risks them arriving out of order over the stdin pipe).
  async function sendAll() {
    const input = $("#console-cmd");
    const lines = input.value.split("\n").map((l) => l.trim()).filter(Boolean);
    if (!lines.length) return;
    for (const line of lines) {
      const ok = await sendOne(line);
      if (!ok) continue; // a dangerous command the user declined - skip it, keep going with the rest
      await new Promise((r) => setTimeout(r, 150));
    }
    input.value = "";
    updateLineCount();
  }

  function updateLineCount() {
    const n = $("#console-cmd").value.split("\n").map((l) => l.trim()).filter(Boolean).length;
    $("#console-line-count").textContent = n > 1 ? `${n} commandes` : "";
  }

  $("#console-send").addEventListener("click", sendAll);
  $("#console-cmd").addEventListener("input", updateLineCount);
  $("#console-cmd").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      sendAll();
    }
  });

  $("#c-start").addEventListener("click", async () => { await api(`/servers/${s.id}/start`, { method: "POST" }); toast("Démarrage…", "success"); });
  $("#c-stop").addEventListener("click", async () => {
    if (!confirm("Arrêter ce serveur ? Les joueurs connectés seront déconnectés.")) return;
    await api(`/servers/${s.id}/stop`, { method: "POST" }); toast("Arrêt demandé.", "success");
  });
  $("#c-kill").addEventListener("click", async () => { if (confirm("Forcer l'arrêt immédiat ? Risque de corruption du monde en cours de sauvegarde.")) { await api(`/servers/${s.id}/kill`, { method: "POST" }); } });
  $("#c-restart").addEventListener("click", async () => {
    if (!confirm("Redémarrer ce serveur ? Les joueurs connectés seront déconnectés puis pourront se reconnecter.")) return;
    try { await api(`/servers/${s.id}/stop`, { method: "POST" }); } catch {}
    setTimeout(async () => { try { await api(`/servers/${s.id}/start`, { method: "POST" }); toast("Redémarrage…", "success"); } catch {} }, 4000);
  });

  // Automated crash triage: disables every mod/plugin, confirms a bare boot
  // works, then re-enables them one at a time to isolate a culprit. Takes a
  // while (one full boot attempt per addon) - progress streams live into
  // this same console via the WebSocket above, the toast at the end just
  // gives the final verdict.
  $("#c-debug").addEventListener("click", async () => {
    if (!confirm(
      "Ce diagnostic va arrêter le serveur s'il tourne, désactiver temporairement tous les mods/plugins, " +
      "puis les tester un par un en redémarrant le serveur à chaque fois (ça peut prendre plusieurs minutes). " +
      "Tout sera remis dans l'état d'origine à la fin. Continuer ?"
    )) return;
    const btn = $("#c-debug");
    btn.disabled = true;
    btn.textContent = "Diagnostic en cours… (voir la console)";
    try {
      const report = await api(`/servers/${s.id}/debug/crash-diagnostic`, { method: "POST" });
      const isProblem = report.culprits.length || !report.baseline_ok || report.combo_suspect;
      toast(report.summary, isProblem ? "error" : "success");
    } catch (e) {
      toast(e.message || "Échec du diagnostic.", "error");
    } finally {
      btn.disabled = false;
      btn.textContent = "🩺 Diagnostiquer un crash";
    }
  });
  $("#c-clear").addEventListener("click", async () => {
    await api(`/servers/${s.id}/console/clear`, { method: "POST" });
    out.textContent = "";
  });
}

// ───────────────────────── files ─────────────────────────

async function renderFiles(path = "") {
  const s = currentServer();
  state.currentPath = path;
  const content = $("#content");
  const entries = await api(`/servers/${s.id}/files?path=${encodeURIComponent(path)}`);
  content.innerHTML = `
    <h1>${t('view.files')} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Chemin actuel : /${escapeHtml(path)}</div>
    <div class="toolbar">
      ${path ? '<button class="btn-ghost" id="f-up">⬆ Dossier parent</button>' : ""}
      <label class="btn-ghost" style="cursor:pointer">📤 Envoyer un fichier<input type="file" id="f-upload" class="hidden"></label>
      <label class="btn-ghost" style="cursor:pointer">📦 Importer une archive .zip<input type="file" id="f-import" accept=".zip" class="hidden"></label>
      <button class="btn-ghost" id="f-export">📥 Exporter ${path ? "ce dossier" : "tout le serveur"} (.zip)</button>
      <button class="btn-ghost" id="f-explorer">🗂 ${t('files.openExplorer')}</button>
    </div>
    <div class="file-browser">
      <div class="file-list" id="file-list"></div>
      <div class="file-editor hidden" id="file-editor">
        <div class="toolbar">
          <b id="editor-name"></b>
          <button class="btn-green" id="editor-save">💾 Enregistrer</button>
        </div>
        <textarea id="editor-content" spellcheck="false"></textarea>
      </div>
    </div>
  `;
  const list = $("#file-list");
  list.innerHTML = entries.map((e) => `
    <div class="file-row" data-name="${escapeHtml(e.name)}" data-dir="${e.is_dir}">
      <span>${e.is_dir ? "📁" : "📄"} ${escapeHtml(e.name)}</span>
      <span style="color:var(--overlay0)">${e.is_dir ? "" : humanSize(e.size_bytes)}</span>
    </div>
  `).join("") || '<div class="empty-state">Dossier vide.</div>';

  $$(".file-row", list).forEach((row) => {
    row.addEventListener("click", async () => {
      const name = row.dataset.name;
      const newPath = path ? `${path}/${name}` : name;
      if (row.dataset.dir === "true") {
        renderFiles(newPath);
      } else {
        openEditor(newPath);
      }
    });
  });

  if (path) $("#f-up").addEventListener("click", () => renderFiles(path.split("/").slice(0, -1).join("/")));

  $("#f-explorer").addEventListener("click", async () => {
    try {
      await api(`/servers/${s.id}/open-folder?path=${encodeURIComponent(path)}`, { method: "POST" });
    } catch {
      toast(t('files.explorerFailed'), "error");
    }
  });

  $("#f-upload").addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const fd = new FormData();
    fd.append("path", path);
    fd.append("file", file);
    await fetch(`/api/servers/${s.id}/files/upload`, { method: "POST", body: fd });
    toast("Fichier envoyé.", "success");
    renderFiles(path);
  });

  $("#f-export").addEventListener("click", () => {
    // Direct navigation (not fetch()) so the browser handles the download
    // and Content-Disposition filename itself, same as any normal file
    // download link.
    window.location.href = `/api/servers/${s.id}/files/export?path=${encodeURIComponent(path)}`;
  });

  $("#f-import").addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    if (!confirm(`Extraire "${file.name}" dans ${path ? "/" + path : "le dossier racine du serveur"} ? Les fichiers de même nom seront écrasés.`)) {
      e.target.value = "";
      return;
    }
    const fd = new FormData();
    fd.append("path", path);
    fd.append("file", file);
    try {
      const res = await fetch(`/api/servers/${s.id}/files/import`, { method: "POST", body: fd });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "Échec de l'import");
      toast(`${data.extracted} fichier(s) extrait(s).`, "success");
      renderFiles(path);
    } catch (err) {
      toast(err.message || "Échec de l'import.", "error");
    }
  });

  async function openEditor(filePath) {
    try {
      const data = await api(`/servers/${s.id}/files/content?path=${encodeURIComponent(filePath)}`);
      $("#file-editor").classList.remove("hidden");
      $("#editor-name").textContent = filePath;
      $("#editor-content").value = data.content;
      $("#editor-save").onclick = async () => {
        await api(`/servers/${s.id}/files/content`, { method: "PUT", body: JSON.stringify({ path: filePath, content: $("#editor-content").value }) });
        toast("Fichier enregistré.", "success");
      };
    } catch {
      toast("Impossible d'ouvrir ce fichier (binaire ou trop volumineux).", "error");
    }
  }
}

function humanSize(bytes) {
  if (bytes < 1024) return `${bytes} o`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} Ko`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} Mo`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} Go`;
}

// ───────────────────────── addons (mods/plugins) ─────────────────────────

async function renderAddons() {
  const s = currentServer();
  const content = $("#content");
  const addons = await api(`/servers/${s.id}/addons`);
  const isModded = ["fabric", "quilt", "forge", "neoforge"].includes(s.loader);
  content.innerHTML = `
    <h1>${isModded ? t('view.mods') : t('view.plugins')} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Activez, désactivez ou supprimez vos ${isModded ? "mods" : "plugins"} installés.</div>
    <div class="toolbar">
      <button class="btn-blue" id="check-updates">🔄 Vérifier les mises à jour</button>
      <button class="btn-mauve" id="boost-perf">⚡ Ajouter les mods/plugins de performance</button>
      <span id="updates-result"></span>
    </div>
    <div class="card">
      <h2>Installés (${addons.length})</h2>
      <div class="mod-list" id="mod-list"></div>
    </div>
    <div class="card">
      <h2>Mods/plugins gérés automatiquement</h2>
      <p style="color:var(--overlay0);font-size:12px;margin-bottom:10px">
        Définissez ici des mods/plugins Modrinth (par ID ou slug de projet, ex. "lithium") à garder dans la bonne
        version pour ce serveur. Rien ne se télécharge tout seul : cliquez sur "Synchroniser" quand vous le voulez.
      </p>
      <div class="form-row" style="display:flex;gap:8px;align-items:flex-end">
        <div style="flex:1"><label>ID ou slug du projet Modrinth</label><input id="ma-project" placeholder="ex. lithium, fabric-api…"></div>
        <div style="flex:1"><label>Nom (affichage)</label><input id="ma-label" placeholder="ex. Lithium"></div>
        <button class="btn-green" id="ma-add">+ Ajouter à la liste</button>
      </div>
      <div id="managed-list" style="margin-top:12px"></div>
      <button class="btn-blue" id="ma-sync" style="margin-top:10px">🔄 Synchroniser maintenant</button>
      <div id="ma-sync-result" style="margin-top:8px;font-size:13px"></div>
    </div>
  `;

  const addonDir = isModded ? "mods" : "plugins";
  const managedIds = new Set((s.managed_addons || []).map((m) => m.project_id));
  const modList = $("#mod-list");
  modList.innerHTML = addons.length ? addons.map((a) => `
    <div class="mod-row">
      <div>
        <div class="name">${escapeHtml(a.file_name)}</div>
        <div class="meta">${humanSize(a.size_bytes)} · ${a.enabled ? "Activé" : "Désactivé"}</div>
      </div>
      <div class="mod-actions">
        <button class="btn-ghost" data-config="${escapeHtml(a.file_name)}" title="${t('addons.configTip')}">${t('addons.config')}</button>
        <button class="btn-ghost" data-track-existing="${escapeHtml(a.file_name)}" title="Repérer ce fichier sur Modrinth et le suivre pour les mises à jour auto">👁 Suivre</button>
        <button class="btn-ghost" data-toggle="${escapeHtml(a.file_name)}">${a.enabled ? t('addons.disable') : t('addons.enable')}</button>
        <button class="btn-red" data-remove="${escapeHtml(a.file_name)}">${t('common.delete')}</button>
      </div>
    </div>
  `).join("") : `<div class="empty-state">Aucun ${isModded ? "mod" : "plugin"} installé. Utilisez le Marketplace pour en ajouter.</div>`;

  $$("[data-toggle]", modList).forEach((b) => b.addEventListener("click", async () => {
    await api(`/servers/${s.id}/addons/${encodeURIComponent(b.dataset.toggle)}/toggle`, { method: "POST" });
    renderAddons();
  }));
  $$("[data-remove]", modList).forEach((b) => b.addEventListener("click", async () => {
    if (!confirm("Supprimer ce fichier ?")) return;
    await api(`/servers/${s.id}/addons/${encodeURIComponent(b.dataset.remove)}`, { method: "DELETE" });
    renderAddons();
  }));
  // "Suivre" identifies an already-installed file via Modrinth (by hash,
  // same mechanism as the update checker) and adds it to the "managed"
  // list below, so files installed by hand (or before that feature
  // existed) can still be kept up to date with "Synchroniser maintenant"
  // without the user needing to know its Modrinth slug/ID.
  $$("[data-track-existing]", modList).forEach((b) => b.addEventListener("click", async () => {
    b.disabled = true;
    const original = b.textContent;
    b.textContent = "Identification…";
    try {
      const updated = await api(`/servers/${s.id}/addons/${encodeURIComponent(b.dataset.trackExisting)}/track`, { method: "POST" });
      state.servers = state.servers.map((sv) => sv.id === updated.id ? updated : sv);
      toast(`${b.dataset.trackExisting} est maintenant suivi (voir "Mods/plugins gérés automatiquement" ci-dessous).`, "success");
      renderAddons();
    } catch (e) {
      toast(e.message || "Fichier non reconnu par Modrinth.", "error");
      b.disabled = false;
      b.textContent = original;
    }
  }));
  // Addons already tracked get the button hidden rather than left
  // clickable-but-redundant.
  $$("[data-track-existing]", modList).forEach((b) => {
    // We don't know an installed file's project_id without asking the
    // server, so this only hides for files whose name obviously matches an
    // already-managed label - a light best-effort dedupe, not exhaustive.
    const name = b.dataset.trackExisting.replace(/\.jar$/, "");
    if ([...managedIds].some((id) => name.toLowerCase().includes(id.toLowerCase()))) {
      b.style.display = "none";
    }
  });

  // "Config" jumps straight into the mods/plugins folder in the Files
  // browser (rather than guessing a per-plugin config path, which varies
  // wildly between plugins) so the user lands one click away from e.g.
  // plugins/EssentialsX/config.yml.
  $$("[data-config]", modList).forEach((b) => b.addEventListener("click", () => {
    state.view = "files";
    $$(".nav-item").forEach((n) => n.classList.toggle("active", n.dataset.view === "files"));
    renderFiles(addonDir);
  }));

  $("#check-updates").addEventListener("click", async () => {
    $("#updates-result").textContent = "Vérification en cours…";
    const updates = await api(`/servers/${s.id}/marketplace/updates`);
    $("#updates-result").textContent = updates.length ? `${updates.length} mise(s) à jour disponible(s).` : "Tout est à jour.";
  });

  $("#boost-perf").addEventListener("click", async () => {
    const btn = $("#boost-perf");
    btn.disabled = true;
    btn.textContent = "Installation en cours…";
    try {
      const results = await api(`/servers/${s.id}/presets/category/performance/install`, { method: "POST" });
      const ok = results.filter((r) => r.ok).length;
      const failed = results.filter((r) => !r.ok);
      toast(
        failed.length
          ? `${ok} installé(s), ${failed.length} échec(s) : ${failed.map((f) => f.label).join(", ")}`
          : `${ok} mod(s)/plugin(s) de performance installés.`,
        failed.length ? "error" : "success"
      );
      renderAddons();
    } catch (e) {
      toast(e.message || "Échec de l'installation.", "error");
      btn.disabled = false;
      btn.textContent = "⚡ Ajouter les mods/plugins de performance";
    }
  });

  renderManagedAddons(s);
}

function renderManagedAddons(s) {
  const list = $("#managed-list");
  const managed = s.managed_addons || [];
  list.innerHTML = managed.length ? managed.map((m) => `
    <div class="mod-row">
      <div class="name">${escapeHtml(m.label || m.project_id)} <span class="meta">(${escapeHtml(m.project_id)})</span></div>
      <button class="btn-red" data-ma-remove="${escapeHtml(m.project_id)}">${t('common.delete')}</button>
    </div>
  `).join("") : `<div class="empty-state">Aucun mod/plugin géré pour l'instant.</div>`;

  $$("[data-ma-remove]", list).forEach((b) => b.addEventListener("click", async () => {
    const updated = await api(`/servers/${s.id}/managed-addons/${encodeURIComponent(b.dataset.maRemove)}`, { method: "DELETE" });
    state.servers = state.servers.map((sv) => sv.id === updated.id ? updated : sv);
    renderManagedAddons(updated);
  }));

  $("#ma-add").addEventListener("click", async () => {
    const project_id = $("#ma-project").value.trim();
    if (!project_id) return;
    const label = $("#ma-label").value.trim() || project_id;
    const updated = await api(`/servers/${s.id}/managed-addons`, { method: "POST", body: JSON.stringify({ project_id, label }) });
    state.servers = state.servers.map((sv) => sv.id === updated.id ? updated : sv);
    $("#ma-project").value = "";
    $("#ma-label").value = "";
    renderManagedAddons(updated);
  });

  $("#ma-sync").addEventListener("click", async () => {
    const btn = $("#ma-sync");
    btn.disabled = true;
    btn.textContent = "Synchronisation…";
    try {
      const results = await api(`/servers/${s.id}/managed-addons/sync`, { method: "POST" });
      const ok = results.filter((r) => r.ok).length;
      const failed = results.filter((r) => !r.ok);
      $("#ma-sync-result").innerHTML = results.length
        ? `${ok} synchronisé(s)${failed.length ? `, ${failed.length} échec(s) : ` + failed.map((f) => `${escapeHtml(f.label)} (${escapeHtml(f.error)})`).join(", ") : "."}`
        : "Rien à synchroniser.";
    } finally {
      btn.disabled = false;
      btn.textContent = "🔄 Synchroniser maintenant";
    }
  });
}

// ───────────────────────── marketplace ─────────────────────────

async function renderMarketplace() {
  const s = currentServer();
  const content = $("#content");
  const projectType = ["fabric", "quilt", "forge", "neoforge"].includes(s.loader) ? "mod" : "plugin";
  content.innerHTML = `
    <h1>${t('view.marketplace')}</h1>
    <div class="subtitle">Recherche intégrée Modrinth, filtrée pour ${s.loader} ${s.mc_version}.</div>
    <div class="toolbar">
      <input id="mk-query" placeholder="Rechercher un mod ou plugin…" style="flex:1;min-width:240px">
      <button class="btn-blue" id="mk-search">Rechercher</button>
    </div>
    <div class="market-grid" id="mk-results"><div class="empty-state">Lancez une recherche pour commencer.</div></div>
  `;

  async function search() {
    const q = $("#mk-query").value;
    const results = $("#mk-results");
    results.innerHTML = '<div class="empty-state">Recherche…</div>';
    const hits = await api(`/marketplace/search?q=${encodeURIComponent(q)}&type=${projectType}&loader=${s.loader === "vanilla" ? "" : encodeURIComponent(s.loader)}&version=${encodeURIComponent(s.mc_version)}`);
    results.innerHTML = hits.length ? hits.map((h) => `
      <div class="market-card">
        <div style="display:flex;gap:8px;align-items:center">
          ${h.icon_url ? `<img src="${h.icon_url}">` : ""}
          <div class="title">${escapeHtml(h.title)}</div>
        </div>
        <div class="desc">${escapeHtml(h.description)}</div>
        <div class="meta" style="font-size:11px;color:var(--overlay0)">${h.downloads.toLocaleString()} téléchargements</div>
        <button class="btn-green" data-install="${h.project_id}">+ Installer</button>
        <button class="btn-ghost" data-track="${h.project_id}" data-label="${escapeHtml(h.title)}">➕ Suivi auto</button>
      </div>
    `).join("") : '<div class="empty-state">Aucun résultat.</div>';

    $$("[data-install]", results).forEach((b) => b.addEventListener("click", async () => {
      b.disabled = true;
      b.textContent = "Installation…";
      try {
        await api(`/servers/${s.id}/marketplace/install`, { method: "POST", body: JSON.stringify({ project_id: b.dataset.install }) });
        toast("Installé avec succès. Redémarrez le serveur pour l'activer.", "success");
        b.textContent = "✓ Installé";
      } catch {
        b.disabled = false;
        b.textContent = "+ Installer";
      }
    }));
    // "Suivi auto" adds the project to this server's managed-addons list
    // (see the Addons tab) so it can be kept up to date with one click
    // later, without re-searching for it.
    $$("[data-track]", results).forEach((b) => b.addEventListener("click", async () => {
      const updated = await api(`/servers/${s.id}/managed-addons`, {
        method: "POST",
        body: JSON.stringify({ project_id: b.dataset.track, label: b.dataset.label }),
      });
      state.servers = state.servers.map((sv) => sv.id === updated.id ? updated : sv);
      b.textContent = "✓ Suivi";
      b.disabled = true;
    }));
  }
  $("#mk-search").addEventListener("click", search);
  $("#mk-query").addEventListener("keydown", (e) => { if (e.key === "Enter") search(); });
}

// ───────────────────────── schematics ─────────────────────────

async function renderSchematics() {
  const s = currentServer();
  const content = $("#content");
  const list = await api(`/servers/${s.id}/schematics`);
  content.innerHTML = `
    <h1>Schematics (WorldEdit / FAWE)</h1>
    <div class="subtitle">Déposez vos fichiers <code>.schem</code> / <code>.schematic</code> — ils seront disponibles côté serveur avec <code>//schem load &lt;nom&gt;</code>.</div>
    <div class="toolbar">
      <label class="btn-blue" style="cursor:pointer">📤 Envoyer un schematic<input type="file" id="sc-upload" class="hidden" accept=".schem,.schematic" multiple></label>
    </div>
    <div class="mod-list" id="sc-list"></div>
  `;
  $("#sc-list").innerHTML = list.length ? list.map((f) => `
    <div class="mod-row">
      <div class="name">${escapeHtml(f.name)}</div>
      <div class="mod-actions">
        <span class="meta">${humanSize(f.size_bytes)}</span>
        <button class="btn-red" data-del-sc="${escapeHtml(f.name)}">${t('common.delete')}</button>
      </div>
    </div>`).join("") : '<div class="empty-state">Aucun schematic pour le moment.</div>';

  $$("[data-del-sc]").forEach((b) => b.addEventListener("click", async () => {
    await api(`/servers/${s.id}/schematics/${encodeURIComponent(b.dataset.delSc)}`, { method: "DELETE" });
    renderSchematics();
  }));

  $("#sc-upload").addEventListener("change", async (e) => {
    const fd = new FormData();
    for (const file of e.target.files) fd.append("file", file);
    await fetch(`/api/servers/${s.id}/schematics`, { method: "POST", body: fd });
    toast("Schematic(s) envoyé(s).", "success");
    renderSchematics();
  });
}

// ───────────────────────── backups ─────────────────────────

async function pollBackupProgress(serverId) {
  const label = $("#bk-progress");
  while (true) {
    let p;
    try { p = await api(`/servers/${serverId}/backups/progress`); } catch { break; }
    if (!p.running) {
      if (label) label.textContent = "";
      break;
    }
    const pct = p.total > 0 ? Math.min(100, Math.round((p.done / p.total) * 100)) : 0;
    if (label) label.textContent = `Sauvegarde en cours… ${pct}% (${p.done}/${p.total} éléments)`;
    await new Promise((r) => setTimeout(r, 600));
  }
}

async function renderBackups() {
  const s = currentServer();
  const content = $("#content");
  const backups = await api(`/servers/${s.id}/backups`);
  content.innerHTML = `
    <h1>${t('view.backups')} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Sauvegardes complètes (mondes, configs, mods/plugins) au format .zip.</div>
    <div class="toolbar">
      <button class="btn-blue" id="bk-create">💾 Créer une sauvegarde maintenant</button>
      <span id="bk-progress" style="font-size:12px;color:var(--subtext1)"></span>
    </div>
    <table>
      <thead><tr><th>Nom</th><th>Date</th><th>Taille</th><th></th></tr></thead>
      <tbody id="bk-rows"></tbody>
    </table>
  `;
  const rows = $("#bk-rows");
  rows.innerHTML = backups.length ? backups.map((b) => `
    <tr>
      <td>${escapeHtml(b.name)}</td>
      <td>${new Date(b.created_at).toLocaleString("fr-FR")}</td>
      <td>${humanSize(b.size_bytes)}</td>
      <td>
        <button class="btn-yellow" data-restore="${escapeHtml(b.name)}">Restaurer</button>
        <button class="btn-red" data-delbk="${escapeHtml(b.name)}">${t('common.delete')}</button>
      </td>
    </tr>`).join("") : '<tr><td colspan="4" class="empty-state">Aucune sauvegarde.</td></tr>';

  $("#bk-create").addEventListener("click", async () => {
    $("#bk-create").disabled = true;
    const status = await api(`/servers/${s.id}/backups`, { method: "POST" });
    toast("Création de la sauvegarde…", "success");
    await pollBackupProgress(s.id);
    $("#bk-create").disabled = false;
    renderBackups();
  });
  $$("[data-restore]", rows).forEach((b) => b.addEventListener("click", async () => {
    if (!confirm("Restaurer cette sauvegarde ? Le contenu actuel du serveur sera remplacé. Le serveur doit être arrêté.")) return;
    await api(`/servers/${s.id}/backups/${encodeURIComponent(b.dataset.restore)}/restore`, { method: "POST" });
    toast("Sauvegarde restaurée.", "success");
  }));
  $$("[data-delbk]", rows).forEach((b) => b.addEventListener("click", async () => {
    await api(`/servers/${s.id}/backups/${encodeURIComponent(b.dataset.delbk)}`, { method: "DELETE" });
    renderBackups();
  }));
}

// ───────────────────────── statistiques ─────────────────────────

function formatDuration(totalSeconds) {
  const d = Math.floor(totalSeconds / 86400);
  const h = Math.floor((totalSeconds % 86400) / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const parts = [];
  if (d) parts.push(`${d} j`);
  if (h) parts.push(`${h} h`);
  if (!d && m) parts.push(`${m} min`);
  return parts.length ? parts.join(" ") : "< 1 min";
}

async function renderStats() {
  const s = currentServer();
  const content = $("#content");
  const hist = await api(`/servers/${s.id}/history`);
  content.innerHTML = `
    <h1>📈 ${t('view.stats')} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Historique de fonctionnement de ce serveur (conservé localement par MCManager).</div>
    <div class="grid">
      <div class="stat-card"><div class="label">Démarrages</div><div class="value">${hist.total_boots}</div></div>
      <div class="stat-card"><div class="label">Temps de fonctionnement total</div><div class="value">${formatDuration(hist.total_uptime_secs)}</div></div>
      <div class="stat-card"><div class="label">Crashs détectés</div><div class="value" style="${hist.total_crashes ? 'color:var(--red)' : ''}">${hist.total_crashes}</div></div>
    </div>
    <div class="card">
      <h2>Sessions récentes</h2>
      <table>
        <thead><tr><th>Démarré</th><th>Arrêté</th><th>Durée</th><th>État</th></tr></thead>
        <tbody>
          ${hist.records.length ? hist.records.map((r) => {
            const start = new Date(r.started_at);
            const end = r.stopped_at ? new Date(r.stopped_at) : null;
            const durationSecs = end ? Math.max(0, Math.round((end - start) / 1000)) : null;
            return `<tr>
              <td>${start.toLocaleString("fr-FR")}</td>
              <td>${end ? end.toLocaleString("fr-FR") : '<span class="meta">en cours</span>'}</td>
              <td>${durationSecs !== null ? formatDuration(durationSecs) : "—"}</td>
              <td>${r.crashed ? '<span class="badge badge-red">Crash</span>' : (end ? '<span class="badge badge-green">Arrêt propre</span>' : '<span class="badge badge-green">Actif</span>')}</td>
            </tr>`;
          }).join("") : '<tr><td colspan="4" class="empty-state">Aucune session enregistrée pour l\'instant.</td></tr>'}
        </tbody>
      </table>
    </div>
  `;
}

// ───────────────────────── liste blanche (whitelist) ─────────────────────────

async function renderWhitelist() {
  const s = currentServer();
  const content = $("#content");
  const status = await api(`/servers/${s.id}/status`);

  let whitelistNames = [];
  try {
    const raw = await api(`/servers/${s.id}/files/content?path=${encodeURIComponent("whitelist.json")}`);
    whitelistNames = JSON.parse(raw.content || "[]").map((e) => e.name).filter(Boolean);
  } catch { /* fichier absent = liste vide, tant pis */ }

  let enforced = false;
  let propsContent = "";
  try {
    const raw = await api(`/servers/${s.id}/files/content?path=${encodeURIComponent("server.properties")}`);
    propsContent = raw.content || "";
    enforced = /^white-list=true/m.test(propsContent);
  } catch { /* pas encore de server.properties */ }

  content.innerHTML = `
    <h1>🛡 ${t('view.whitelist')} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Seuls les joueurs listés ici pourront rejoindre si la liste blanche est activée.</div>
    <div class="card">
      <label style="display:flex;align-items:center;gap:8px">
        <input type="checkbox" id="wl-enforce" style="width:auto" ${enforced ? "checked" : ""}>
        Activer la liste blanche sur ce serveur
      </label>
      <p style="color:var(--overlay0);font-size:12px;margin-top:6px">${status.running ? "Le serveur est en cours d'exécution : le changement est appliqué immédiatement." : "Le serveur est arrêté : le changement sera actif au prochain démarrage."}</p>
    </div>
    <div class="card">
      <h2>Ajouter un joueur</h2>
      <div style="display:flex;gap:8px">
        <input id="wl-add-name" placeholder="Pseudo Minecraft" ${status.running ? "" : "disabled"}>
        <button class="btn-green" id="wl-add" ${status.running ? "" : "disabled"}>+ Ajouter</button>
      </div>
      ${status.running ? "" : '<p style="color:var(--overlay0);font-size:12px;margin-top:6px">Démarrez le serveur pour ajouter un joueur (la résolution du pseudo en UUID se fait via le serveur lui-même).</p>'}
    </div>
    <div class="card">
      <h2>Joueurs autorisés (${whitelistNames.length})</h2>
      <div id="wl-list">
        ${whitelistNames.length ? whitelistNames.map((n) => `
          <div class="mod-row">
            <div class="name">${escapeHtml(n)}</div>
            <button class="btn-red" data-wl-remove="${escapeHtml(n)}">${t('common.delete')}</button>
          </div>
        `).join("") : '<div class="empty-state">Aucun joueur dans la liste blanche.</div>'}
      </div>
    </div>
  `;

  $("#wl-enforce").addEventListener("change", async (e) => {
    const on = e.target.checked;
    if (status.running) {
      await api(`/servers/${s.id}/command`, { method: "POST", body: JSON.stringify({ cmd: `whitelist ${on ? "on" : "off"}` }) });
    } else {
      const lines = propsContent.split("\n").filter((l) => !l.startsWith("white-list="));
      lines.push(`white-list=${on}`);
      await api(`/servers/${s.id}/files/content`, { method: "PUT", body: JSON.stringify({ path: "server.properties", content: lines.join("\n") }) });
    }
    toast(on ? "Liste blanche activée." : "Liste blanche désactivée.", "success");
  });

  $("#wl-add").addEventListener("click", async () => {
    const name = $("#wl-add-name").value.trim();
    if (!name) return;
    await api(`/servers/${s.id}/command`, { method: "POST", body: JSON.stringify({ cmd: `whitelist add ${name}` }) });
    toast(`${name} ajouté à la liste blanche.`, "success");
    setTimeout(renderWhitelist, 500); // laisse le serveur écrire whitelist.json avant de relire
  });

  $$("[data-wl-remove]").forEach((b) => b.addEventListener("click", async () => {
    const name = b.dataset.wlRemove;
    if (status.running) {
      await api(`/servers/${s.id}/command`, { method: "POST", body: JSON.stringify({ cmd: `whitelist remove ${name}` }) });
      setTimeout(renderWhitelist, 500);
    } else {
      try {
        const raw = await api(`/servers/${s.id}/files/content?path=${encodeURIComponent("whitelist.json")}`);
        const list = JSON.parse(raw.content || "[]").filter((e) => e.name !== name);
        await api(`/servers/${s.id}/files/content`, { method: "PUT", body: JSON.stringify({ path: "whitelist.json", content: JSON.stringify(list, null, 2) }) });
        renderWhitelist();
      } catch {
        toast("Impossible de modifier whitelist.json.", "error");
      }
    }
  }));
}

// ───────────────────────── propriétés du serveur (server.properties) ─────────────────────────

// Champs connus avec un contrôle adapté ; tout le reste du fichier
// (commentaires, clés non listées ici) est préservé tel quel à l'enregistrement.
const KNOWN_PROPERTIES = [
  { key: "motd", label: "Message d'accueil (MOTD)", type: "text" },
  { key: "difficulty", label: "Difficulté", type: "select", options: ["peaceful", "easy", "normal", "hard"] },
  { key: "gamemode", label: "Mode de jeu", type: "select", options: ["survival", "creative", "adventure", "spectator"] },
  { key: "max-players", label: "Joueurs maximum", type: "number" },
  { key: "view-distance", label: "Distance de vue (chunks)", type: "number" },
  { key: "simulation-distance", label: "Distance de simulation (chunks)", type: "number" },
  { key: "spawn-protection", label: "Rayon de protection du spawn", type: "number" },
  { key: "pvp", label: "PvP activé", type: "bool" },
  { key: "hardcore", label: "Mode hardcore", type: "bool" },
  { key: "online-mode", label: "Mode en ligne (vérification Mojang)", type: "bool" },
  { key: "allow-flight", label: "Autoriser le vol", type: "bool" },
  { key: "enable-command-block", label: "Activer les blocs de commande", type: "bool" },
  { key: "allow-nether", label: "Autoriser le Nether", type: "bool" },
  { key: "spawn-monsters", label: "Faire apparaître des monstres", type: "bool" },
  { key: "level-seed", label: "Seed du monde (à la création uniquement)", type: "text" },
];

function parseProperties(content) {
  const map = {};
  for (const line of content.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const idx = trimmed.indexOf("=");
    if (idx === -1) continue;
    map[trimmed.slice(0, idx)] = trimmed.slice(idx + 1);
  }
  return map;
}

function applyPropertiesChanges(content, changes) {
  const lines = content.split("\n");
  const seen = new Set();
  const updated = lines.map((line) => {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) return line;
    const idx = trimmed.indexOf("=");
    if (idx === -1) return line;
    const key = trimmed.slice(0, idx);
    if (Object.prototype.hasOwnProperty.call(changes, key)) {
      seen.add(key);
      return `${key}=${changes[key]}`;
    }
    return line;
  });
  for (const [key, value] of Object.entries(changes)) {
    if (!seen.has(key)) updated.push(`${key}=${value}`);
  }
  return updated.join("\n");
}

async function renderProperties() {
  const s = currentServer();
  const content = $("#content");
  let raw = "";
  try {
    const res = await api(`/servers/${s.id}/files/content?path=${encodeURIComponent("server.properties")}`);
    raw = res.content || "";
  } catch {
    content.innerHTML = `<h1>📝 ${t('view.properties')} — ${escapeHtml(s.name)}</h1><div class="empty-state">${t('view.propertiesMissing')}</div>`;
    return;
  }
  const props = parseProperties(raw);
  const status = await api(`/servers/${s.id}/status`);

  content.innerHTML = `
    <h1>📝 ${t('view.properties')} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Réglages les plus courants de server.properties, avec des contrôles adaptés plutôt que du texte brut. ${status.running ? "Un redémarrage est nécessaire pour appliquer les changements." : ""}</div>
    <div class="card">
      <div class="form-grid">
        ${KNOWN_PROPERTIES.map((p) => {
          const val = props[p.key] ?? "";
          if (p.type === "bool") {
            return `<div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" class="prop-field" data-key="${p.key}" data-type="bool" style="width:auto" ${val === "true" ? "checked" : ""}> ${p.label}</label></div>`;
          }
          if (p.type === "select") {
            return `<div class="form-row"><label>${p.label}</label><select class="prop-field" data-key="${p.key}" data-type="select">${p.options.map((o) => `<option value="${o}" ${val === o ? "selected" : ""}>${o}</option>`).join("")}</select></div>`;
          }
          return `<div class="form-row"><label>${p.label}</label><input class="prop-field" data-key="${p.key}" data-type="${p.type}" type="${p.type === "number" ? "number" : "text"}" value="${escapeHtml(val)}"></div>`;
        }).join("")}
      </div>
      <button class="btn-green" id="props-save">${t('common.save')}</button>
    </div>
    <div class="card">
      <h2>Fichier complet (avancé)</h2>
      <p style="color:var(--overlay0);font-size:12px;margin-bottom:8px">Pour les clés non listées ci-dessus. Modifier ici écrase les valeurs saisies plus haut si elles se chevauchent — préférez le formulaire pour les réglages courants.</p>
      <textarea id="props-raw" style="width:100%;min-height:260px;font-family:Consolas,monospace;font-size:12.5px">${escapeHtml(raw)}</textarea>
      <button class="btn-blue" id="props-save-raw" style="margin-top:8px">Enregistrer le fichier complet</button>
    </div>
  `;

  $("#props-save").addEventListener("click", async () => {
    const changes = {};
    $$(".prop-field").forEach((el) => {
      const key = el.dataset.key;
      if (el.dataset.type === "bool") changes[key] = el.checked ? "true" : "false";
      else changes[key] = el.value;
    });
    const updated = applyPropertiesChanges(raw, changes);
    await api(`/servers/${s.id}/files/content`, { method: "PUT", body: JSON.stringify({ path: "server.properties", content: updated }) });
    toast("Propriétés enregistrées.", "success");
    renderProperties();
  });

  $("#props-save-raw").addEventListener("click", async () => {
    await api(`/servers/${s.id}/files/content`, { method: "PUT", body: JSON.stringify({ path: "server.properties", content: $("#props-raw").value }) });
    toast("Fichier server.properties enregistré.", "success");
    renderProperties();
  });
}

// ───────────────────────── network (playit.gg) ─────────────────────────

async function renderNetwork() {
  const content = $("#content");
  const status = await api("/playit/status");
  content.innerHTML = `
    <h1>${t('view.network')} — playit.gg</h1>
    <div class="subtitle">Exposez votre serveur sur Internet sans configurer votre routeur (pas d'ouverture de ports nécessaire).</div>
    <div class="card">
      <h2>Statut de l'agent</h2>
      <p>${status.running ? '<span class="badge badge-green">En cours d’exécution</span>' : '<span class="badge badge-red">Arrêté</span>'}</p>
      <p style="color:var(--subtext1);font-size:12px">${status.path ? "Binaire : " + escapeHtml(status.path) : "playit-agent n'est pas encore installé."}</p>
      <div class="toolbar">
        <button class="btn-blue" id="pl-download">⬇ Installer / mettre à jour l'agent (téléchargé par MCManager)</button>
        <button class="btn-ghost" id="pl-local">🔎 Utiliser mon installation locale de playit</button>
        <button class="btn-green" id="pl-start" ${status.running ? "disabled" : ""}>▶ Démarrer</button>
        <button class="btn-red" id="pl-stop" ${status.running ? "" : "disabled"}>⏹ Arrêter</button>
      </div>
      <div class="console" id="pl-console" style="height:220px;margin-top:10px"></div>
    </div>
    <div class="card doc-block">
      <h3>Mini-tuto : connecter votre serveur avec playit.gg</h3>
      <ol>
        <li>Cliquez sur <b>Installer / mettre à jour l'agent</b> (télécharge le binaire officiel depuis GitHub).</li>
        <li>Cliquez sur <b>Démarrer</b> — la console ci-dessus affichera un lien du type <code>https://playit.gg/claim/XXXXX</code>.</li>
        <li>Ouvrez ce lien dans votre navigateur et connectez-vous (ou créez un compte gratuit) pour "réclamer" l'agent.</li>
        <li>Sur le site playit.gg, créez un tunnel de type <b>Minecraft Java</b> pointant vers le port local de votre serveur (visible dans l'onglet Serveurs).</li>
        <li>playit.gg vous donne une adresse publique (ex: <code>votre-nom.playit.gg</code>) à partager à vos amis.</li>
      </ol>
      <p>Astuce : gardez l'agent démarré tant que vous voulez que le serveur reste joignable depuis l'extérieur.</p>
    </div>
  `;

  const pw = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/playit/ws`);
  const out = $("#pl-console");
  pw.onmessage = (ev) => { out.textContent += ev.data + "\n"; out.scrollTop = out.scrollHeight; };

  $("#pl-download").addEventListener("click", async () => {
    toast("Téléchargement de playit-agent…", "success");
    await api("/playit/download", { method: "POST" });
    toast("playit-agent installé.", "success");
    renderNetwork();
  });
  $("#pl-local").addEventListener("click", async () => {
    const result = await api("/playit/detect-local", { method: "POST" });
    if (result.found) {
      toast(`Installation locale trouvée : ${result.path}`, "success");
      renderNetwork();
    } else {
      toast("Aucune installation locale de playit trouvée (essayez d'abord curl -SsL https://playit.gg/install.sh | bash).", "error");
    }
  });
  $("#pl-start").addEventListener("click", async () => { await api("/playit/start", { method: "POST" }); renderNetwork(); });
  $("#pl-stop").addEventListener("click", async () => { await api("/playit/stop", { method: "POST" }); renderNetwork(); });
}

// ───────────────────────── controle a distance (piloter une instance mcmanager-headless) ─────────────────────────
//
// The browser only ever talks plain HTTP to this app's own backend - all
// the RSA/AES encryption to reach the actual remote instance happens
// server-side (see src/remote.rs and the /api/remote/* routes).

let remoteSelectedTarget = null;

async function renderRemote() {
  const content = $("#content");
  const targets = await api("/remote/targets");
  if (!remoteSelectedTarget || !targets.some((t) => t.label === remoteSelectedTarget)) {
    remoteSelectedTarget = targets[0]?.label || null;
  }

  content.innerHTML = `
    <h1>🖧 ${t('view.remote')}</h1>
    <div class="subtitle">Pilotez une autre instance MCManager (mode <code>mcmanager-headless</code>, typiquement sur un VPS) depuis cette interface — connexion chiffrée et authentifiée par échange de clés RSA.</div>

    <div class="card">
      <h2>Jumeler une nouvelle instance</h2>
      <p style="color:var(--overlay0);font-size:12px;margin-bottom:10px">
        Sur la machine distante : <code>mcmanager-headless</code> → <code>remote enable &lt;port&gt;</code> puis <code>remote pairing-code</code> pour obtenir un code à usage unique (valide 10 minutes). Vérifiez que l'empreinte affichée ici correspond bien à celle donnée là-bas avant de valider.
      </p>
      <div class="form-grid">
        <div class="form-row"><label>Adresse (hôte:port)</label><input id="rm-host" placeholder="192.168.1.42:7778"></div>
        <div class="form-row"><label>Nom pour cette instance</label><input id="rm-label" placeholder="mon-vps"></div>
        <div class="form-row"><label>Code de jumelage</label><input id="rm-code" placeholder="03847680"></div>
      </div>
      <button class="btn-green" id="rm-pair">🔗 Jumeler</button>
    </div>

    <div class="card">
      <h2>Instances jumelées</h2>
      ${targets.length ? `
        <div class="form-row"><label>Instance</label>
          <select id="rm-target-select">
            ${targets.map((t) => `<option value="${escapeHtml(t.label)}" ${t.label === remoteSelectedTarget ? "selected" : ""}>${escapeHtml(t.label)} (${escapeHtml(t.host)})</option>`).join("")}
          </select>
        </div>
        <button class="btn-red" id="rm-forget">Oublier cette instance</button>
      ` : '<div class="empty-state">Aucune instance jumelée pour l\'instant.</div>'}
    </div>

    ${remoteSelectedTarget ? `
    <div class="card">
      <h2>Serveurs sur "${escapeHtml(remoteSelectedTarget)}"</h2>
      <div id="rm-servers"><div class="empty-state">Chargement…</div></div>
    </div>
    <div class="card">
      <h2>Envoyer un serveur local vers "${escapeHtml(remoteSelectedTarget)}"</h2>
      <p style="color:var(--overlay0);font-size:12px;margin-bottom:10px">Copie le dossier complet du serveur choisi vers l'instance distante et l'y enregistre comme nouveau serveur. Adapté à des serveurs de taille raisonnable — un très gros monde peut prendre du temps (un seul transfert, non repris en cas de coupure).</p>
      <div style="display:flex;gap:8px">
        <select id="rm-deploy-select" style="flex:1">
          ${state.servers.map((s) => `<option value="${s.id}">${escapeHtml(s.name)} (${s.loader} ${s.mc_version})</option>`).join("")}
        </select>
        <button class="btn-mauve" id="rm-deploy">📤 Envoyer</button>
      </div>
      <div id="rm-deploy-result" style="margin-top:8px;font-size:13px"></div>
    </div>
    ` : ""}
  `;

  $("#rm-pair").addEventListener("click", async () => {
    const host = $("#rm-host").value.trim();
    const label = $("#rm-label").value.trim();
    const code = $("#rm-code").value.trim();
    if (!host || !label || !code) { toast("Renseignez l'adresse, le nom et le code.", "error"); return; }
    try {
      await api("/remote/targets", { method: "POST", body: JSON.stringify({ host, label, code }) });
      toast(`Jumelé avec "${label}".`, "success");
      remoteSelectedTarget = label;
      renderRemote();
    } catch (e) {
      toast(e.message || "Échec du jumelage.", "error");
    }
  });

  if (targets.length) {
    $("#rm-target-select").addEventListener("change", (e) => { remoteSelectedTarget = e.target.value; renderRemote(); });
    $("#rm-forget").addEventListener("click", async () => {
      if (!confirm(`Oublier l'instance "${remoteSelectedTarget}" ?`)) return;
      await api(`/remote/targets/${encodeURIComponent(remoteSelectedTarget)}`, { method: "DELETE" });
      remoteSelectedTarget = null;
      renderRemote();
    });
  }

  if (!remoteSelectedTarget) return;

  const serversEl = $("#rm-servers");
  try {
    const result = await api(`/remote/${encodeURIComponent(remoteSelectedTarget)}/call`, { method: "POST", body: JSON.stringify({ action: "list" }) });
    const servers = result.servers || [];
    serversEl.innerHTML = servers.length ? servers.map((s) => `
      <div class="mod-row">
        <div>
          <div class="name">${escapeHtml(s.name)}</div>
          <div class="meta">${escapeHtml(s.loader)} · ${escapeHtml(s.mc_version)} · ${s.running ? '<span class="badge badge-green">En ligne</span>' : '<span class="badge badge-red">Arrêté</span>'}</div>
        </div>
        <div class="mod-actions">
          <button class="btn-green" data-rm-start="${s.id}" ${s.running ? "disabled" : ""}>▶</button>
          <button class="btn-red" data-rm-stop="${s.id}" ${s.running ? "" : "disabled"}>⏹</button>
          <button class="btn-ghost" data-rm-restart="${s.id}">⟳</button>
        </div>
      </div>
    `).join("") : '<div class="empty-state">Aucun serveur enregistré sur cette instance.</div>';

    const callRemote = async (action, id) => {
      try {
        await api(`/remote/${encodeURIComponent(remoteSelectedTarget)}/call`, { method: "POST", body: JSON.stringify({ action, server_id: id }) });
        toast("Commande envoyée.", "success");
        renderRemote();
      } catch (e) {
        toast(e.message || "Échec.", "error");
      }
    };
    $$("[data-rm-start]", serversEl).forEach((b) => b.addEventListener("click", () => callRemote("start", b.dataset.rmStart)));
    $$("[data-rm-stop]", serversEl).forEach((b) => b.addEventListener("click", () => callRemote("stop", b.dataset.rmStop)));
    $$("[data-rm-restart]", serversEl).forEach((b) => b.addEventListener("click", () => callRemote("restart", b.dataset.rmRestart)));
  } catch (e) {
    serversEl.innerHTML = `<div class="empty-state">Impossible de contacter cette instance : ${escapeHtml(e.message || "erreur inconnue")}</div>`;
  }

  $("#rm-deploy")?.addEventListener("click", async () => {
    const serverId = $("#rm-deploy-select").value;
    if (!serverId) return;
    const btn = $("#rm-deploy");
    btn.disabled = true;
    btn.textContent = "Envoi en cours…";
    try {
      const result = await api(`/remote/${encodeURIComponent(remoteSelectedTarget)}/deploy/${serverId}`, { method: "POST" });
      $("#rm-deploy-result").textContent = result.server_id ? `Envoyé avec succès (id distant : ${result.server_id}).` : (result.error || "Terminé.");
      toast("Serveur envoyé.", "success");
      renderRemote();
    } catch (e) {
      $("#rm-deploy-result").textContent = e.message || "Échec de l'envoi.";
      toast(e.message || "Échec de l'envoi.", "error");
    } finally {
      btn.disabled = false;
      btn.textContent = "📤 Envoyer";
    }
  });
}

// ───────────────────────── docs ─────────────────────────

// ───────────────────────── assistant IA ─────────────────────────

state.aiChatHistory = state.aiChatHistory || [];

async function renderAssistant() {
  const content = $("#content");
  const s = currentServer();
  let cfg;
  try {
    cfg = await api("/ai/config");
  } catch {
    cfg = { provider: "anthropic", model: "", ollama_base_url: "", omniroute_base_url: "", has_key: false, masked_key: "" };
  }

  content.innerHTML = `
    <h1>🤖 ${t('view.assistant')}</h1>
    <p class="subtitle">Suggestions personnalisées sur quoi ajouter, modifier ou réparer sur votre serveur. Votre clé API reste stockée localement et est envoyée uniquement au fournisseur choisi — jamais à un serveur MCManager.</p>
    <div class="card">
      <h2>Fournisseur</h2>
      <div class="form-grid">
        <div class="form-row">
          <label>Fournisseur</label>
          <select id="ai-provider">
            <option value="anthropic" ${cfg.provider === "anthropic" ? "selected" : ""}>Anthropic (Claude)</option>
            <option value="openai" ${cfg.provider === "openai" ? "selected" : ""}>OpenAI (GPT)</option>
            <option value="gemini" ${cfg.provider === "gemini" ? "selected" : ""}>Google Gemini</option>
            <option value="ollama" ${cfg.provider === "ollama" ? "selected" : ""}>Ollama (local)</option>
            <option value="omniroute" ${cfg.provider === "omniroute" ? "selected" : ""}>OmniRoute (passerelle multi-fournisseurs)</option>
          </select>
        </div>
        <div class="form-row" id="ai-key-row">
          <label>Clé API ${cfg.has_key ? `<span class="meta">(actuelle : ${escapeHtml(cfg.masked_key)})</span>` : ""}</label>
          <input id="ai-key" type="password" placeholder="${cfg.has_key ? "Laisser vide pour conserver la clé actuelle" : "Collez votre clé API ici"}">
        </div>
        <div class="form-row" id="ai-ollama-row" style="${cfg.provider === "ollama" ? "" : "display:none"}">
          <label>URL Ollama local</label>
          <input id="ai-ollama-url" placeholder="http://127.0.0.1:11434" value="${escapeHtml(cfg.ollama_base_url || "")}">
        </div>
        <div class="form-row" id="ai-omniroute-row" style="${cfg.provider === "omniroute" ? "" : "display:none"}">
          <label>URL OmniRoute</label>
          <input id="ai-omniroute-url" placeholder="http://127.0.0.1:20128/v1" value="${escapeHtml(cfg.omniroute_base_url || "")}">
          <p style="color:var(--overlay0);font-size:12px;margin-top:4px">
            Auto-hébergé (<a href="https://github.com/diegosouzapw/OmniRoute" target="_blank" rel="noopener">github.com/diegosouzapw/OmniRoute</a>,
            <a href="https://omniroute.online/" target="_blank" rel="noopener">omniroute.online</a>) — passerelle vers 300+ fournisseurs (Claude, GPT, Gemini, Kimi, DeepSeek...)
            derrière une seule clé. Par défaut sur l'adresse locale du tableau de bord ; changez l'URL si vous faites tourner OmniRoute ailleurs. La clé API se récupère dans son Dashboard → Endpoints.
          </p>
        </div>
        <div class="form-row">
          <label>Modèle</label>
          <div style="display:flex;gap:8px">
            <select id="ai-model" style="flex:1"><option value="${escapeHtml(cfg.model || "")}">${escapeHtml(cfg.model || "(par défaut)")}</option></select>
            <button class="btn-ghost" id="ai-load-models" type="button">🔄 Détecter les modèles</button>
          </div>
        </div>
      </div>
      <button class="btn-green" id="ai-save">${t('common.save')}</button>
      <p style="color:var(--overlay0);font-size:12px;margin-top:8px">
        La clé est chiffrée sur disque (AES-256-GCM) avec une clé de chiffrement générée localement et stockée séparément, accès restreint au propriétaire du compte. C'est une vraie protection contre une copie/sauvegarde accidentelle du seul fichier de config, mais pas l'équivalent d'un trousseau système : la clé de déchiffrement reste sur la même machine.
        Pour Ollama local, aucune clé n'est nécessaire ; l'assistant peut alors chercher sur le web (DuckDuckGo) et lire des pages pour vous répondre.
      </p>
    </div>
    <div class="card" style="display:flex;flex-direction:column;flex:1;min-height:360px">
      <h2>Discussion ${s ? `— <span class="meta">${escapeHtml(s.name)}</span>` : '<span class="meta">(aucun serveur sélectionné — conseils génériques)</span>'}</h2>
      <div id="ai-chat-log" class="console" style="flex:1"></div>
      <div class="console-input-row">
        <input id="ai-chat-input" placeholder="Ex : Comment réduire le lag ? Quel plugin pour la protection anti-grief ?">
        <button class="btn-blue" id="ai-chat-send">Envoyer</button>
      </div>
    </div>
  `;

  $("#ai-provider").addEventListener("change", (e) => {
    $("#ai-ollama-row").style.display = e.target.value === "ollama" ? "" : "none";
    $("#ai-omniroute-row").style.display = e.target.value === "omniroute" ? "" : "none";
    $("#ai-key-row").style.display = e.target.value === "ollama" ? "none" : "";
  });

  // Auto-detect provider from the shape of a pasted key (sk-ant-... /
  // sk-... / AIza...), mirroring the same heuristic the backend uses.
  // OmniRoute keys have no fixed public prefix, so they aren't auto-detected
  // this way - pick "OmniRoute" from the dropdown manually.
  $("#ai-key").addEventListener("input", (e) => {
    const k = e.target.value.trim();
    let detected = null;
    if (k.startsWith("sk-ant-")) detected = "anthropic";
    else if (k.startsWith("sk-")) detected = "openai";
    else if (k.startsWith("AIza")) detected = "gemini";
    if (detected) {
      $("#ai-provider").value = detected;
      $("#ai-provider").dispatchEvent(new Event("change"));
    }
  });

  $("#ai-load-models").addEventListener("click", async () => {
    const btn = $("#ai-load-models");
    btn.disabled = true;
    btn.textContent = "Détection…";
    try {
      // Save current provider/key/url first so the backend can use them to query models.
      await api("/ai/config", { method: "POST", body: JSON.stringify({
        provider: $("#ai-provider").value,
        api_key: $("#ai-key").value,
        model: $("#ai-model").value || "",
        ollama_base_url: $("#ai-ollama-url").value || "",
        omniroute_base_url: $("#ai-omniroute-url").value || "",
      }) });
      const models = await api("/ai/models");
      const sel = $("#ai-model");
      const current = sel.value;
      sel.innerHTML = models.length
        ? models.map((m) => `<option value="${escapeHtml(m)}" ${m === current ? "selected" : ""}>${escapeHtml(m)}</option>`).join("")
        : `<option value="">(aucun modèle détecté)</option>`;
      toast(models.length ? `${models.length} modèle(s) trouvé(s).` : "Aucun modèle détecté.", models.length ? "success" : "error");
    } catch (e) {
      toast(e.message || "Échec de la détection des modèles.", "error");
    } finally {
      btn.disabled = false;
      btn.textContent = "🔄 Détecter les modèles";
    }
  });

  $("#ai-save").addEventListener("click", async () => {
    try {
      await api("/ai/config", { method: "POST", body: JSON.stringify({
        provider: $("#ai-provider").value,
        api_key: $("#ai-key").value,
        model: $("#ai-model").value || "",
        ollama_base_url: $("#ai-ollama-url").value || "",
        omniroute_base_url: $("#ai-omniroute-url").value || "",
      }) });
      toast("Configuration de l'assistant enregistrée.", "success");
      renderAssistant();
    } catch (e) {
      toast(e.message || "Échec de l'enregistrement.", "error");
    }
  });

  renderAiChatLog();
  $("#ai-chat-send").addEventListener("click", sendAiChat);
  $("#ai-chat-input").addEventListener("keydown", (e) => { if (e.key === "Enter") sendAiChat(); });
}

// ───────────────────────── markdown minimal (chat IA) ─────────────────────────

/// Small, self-contained Markdown → HTML converter for the AI chat bubbles -
/// no CDN dependency, just the subset an LLM actually uses in practice:
/// fenced code blocks, inline code, bold/italic, headers, bullet/numbered
/// lists, links. Input is escaped first, so nothing in a response (however
/// the provider phrases it) can inject markup.
function renderMarkdownLite(text) {
  const codeBlocks = [];
  // Pull fenced code blocks out first so their content isn't mangled by
  // the later inline-formatting passes (a `**` inside a code sample
  // shouldn't turn into bold).
  let working = escapeHtml(text).replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => {
    codeBlocks.push(`<pre class="md-code"><code>${code.replace(/\n$/, "")}</code></pre>`);
    return `\u0000CODEBLOCK${codeBlocks.length - 1}\u0000`;
  });

  working = working
    .replace(/^### (.*)$/gm, "<h4>$1</h4>")
    .replace(/^## (.*)$/gm, "<h4>$1</h4>")
    .replace(/^# (.*)$/gm, "<h4>$1</h4>")
    .replace(/`([^`\n]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*\n]+)\*\*/g, "<b>$1</b>")
    .replace(/(?<!\*)\*([^*\n]+)\*(?!\*)/g, "<i>$1</i>")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');

  // Bullet/numbered lists: group consecutive matching lines into one
  // <ul>/<ol> rather than wrapping each line individually.
  const lines = working.split("\n");
  const out = [];
  let listType = null;
  for (const line of lines) {
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line);
    const numbered = /^\s*\d+\.\s+(.*)$/.exec(line);
    if (bullet || numbered) {
      const type = bullet ? "ul" : "ol";
      if (listType !== type) {
        if (listType) out.push(`</${listType}>`);
        out.push(`<${type}>`);
        listType = type;
      }
      out.push(`<li>${(bullet || numbered)[1]}</li>`);
    } else {
      if (listType) { out.push(`</${listType}>`); listType = null; }
      out.push(line);
    }
  }
  if (listType) out.push(`</${listType}>`);

  let html = out.join("\n").replace(/\n/g, "<br>").replace(/(<\/(ul|ol|li|h4)>)<br>/g, "$1");
  html = html.replace(/\u0000CODEBLOCK(\d+)\u0000/g, (_, i) => codeBlocks[Number(i)]);
  return html;
}

function renderAiChatLog() {
  const log = $("#ai-chat-log");
  if (!log) return;
  log.innerHTML = state.aiChatHistory.length
    ? state.aiChatHistory.map((m) => `<div class="ai-msg ai-msg-${m.role}"><b>${m.role === "user" ? "Vous" : "Assistant"} :</b> ${m.role === "assistant" ? renderMarkdownLite(m.content) : escapeHtml(m.content).replace(/\n/g, "<br>")}</div>`).join("")
    : `<div class="empty-state">Posez une question sur votre serveur — mods à installer, réglages de perf, pourquoi ça lag...</div>`;
  log.scrollTop = log.scrollHeight;
}

async function sendAiChat() {
  const input = $("#ai-chat-input");
  const message = input.value.trim();
  if (!message) return;
  input.value = "";
  state.aiChatHistory.push({ role: "user", content: message });
  renderAiChatLog();
  const log = $("#ai-chat-log");
  log.innerHTML += `<div class="empty-state" id="ai-typing">L'assistant réfléchit…</div>`;
  log.scrollTop = log.scrollHeight;
  try {
    const s = currentServer();
    const res = await api("/ai/chat", { method: "POST", body: JSON.stringify({
      message,
      history: state.aiChatHistory.slice(0, -1),
      server_id: s ? s.id : null,
    }) });
    state.aiChatHistory.push({ role: "assistant", content: res.reply });
  } catch (e) {
    state.aiChatHistory.push({ role: "assistant", content: `⚠ ${e.message || "Erreur lors de la requête à l'assistant."}` });
  }
  renderAiChatLog();
}

function renderDocs() {
  const content = $("#content");
  const sections = [1, 2, 3, 4, 5, 6].map((n) => `
      <h3>${t(`docs.s${n}_h`)}</h3>
      <p>${t(`docs.s${n}_p`)}</p>`).join("");
  content.innerHTML = `
    <h1>${t('docs.title')}</h1>
    <div class="card doc-block">${sections}
    </div>
  `;
}

// ───────────────────────── settings ─────────────────────────

async function renderSettings() {
  const content = $("#content");
  const cfg = await api("/settings");
  const s = currentServer();
  const appearance = getConsoleAppearance();
  let ntfyCfg;
  try { ntfyCfg = await api("/ntfy/config"); } catch { ntfyCfg = { enabled: false, server_url: "", topic: "", has_token: false, notify_crash: true, notify_backup: true, notify_scheduled_restart: true, notify_auto_stop: true, notify_player_join_leave: false }; }
  content.innerHTML = `
    <h1>${t('view.settings')}</h1>
    <div class="card">
      <h2>${t('common.language')}</h2>
      <div class="form-grid">
        <div class="form-row">
          <label>${t('settings.language')}</label>
          <select id="st-lang">
            <option value="fr" ${state.lang === "fr" ? "selected" : ""}>Français</option>
            <option value="en" ${state.lang === "en" ? "selected" : ""}>English</option>
            <option value="es" ${state.lang === "es" ? "selected" : ""}>Español</option>
          </select>
        </div>
      </div>
    </div>
    <div class="card">
      <h2>Apparence de la console</h2>
      <div class="form-grid">
        <div class="form-row">
          <label>Taille de police (${appearance.fontSize}px)</label>
          <input id="ap-fontsize" type="range" min="10" max="20" step="0.5" value="${appearance.fontSize}">
        </div>
        <div class="form-row">
          <label>Police</label>
          <select id="ap-fontfamily">
            <option value="Consolas, 'Courier New', monospace" ${appearance.fontFamily.startsWith("Consolas") ? "selected" : ""}>Consolas</option>
            <option value="'Courier New', monospace" ${appearance.fontFamily.startsWith("'Courier New'") ? "selected" : ""}>Courier New</option>
            <option value="'Cascadia Code', Consolas, monospace" ${appearance.fontFamily.startsWith("'Cascadia") ? "selected" : ""}>Cascadia Code</option>
            <option value="'JetBrains Mono', Consolas, monospace" ${appearance.fontFamily.startsWith("'JetBrains") ? "selected" : ""}>JetBrains Mono</option>
            <option value="monospace" ${appearance.fontFamily === "monospace" ? "selected" : ""}>Monospace système</option>
          </select>
        </div>
      </div>
      <p style="color:var(--overlay0);font-size:12px">S'applique immédiatement, à toutes les consoles (jeu et playit.gg). Enregistré sur cet appareil uniquement.</p>
    </div>
    <div class="card">
      <h2>🔔 Notifications (ntfy)</h2>
      <p style="color:var(--overlay0);font-size:12px;margin-bottom:10px">
        Pousse des notifications vers <a href="https://ntfy.sh" target="_blank" rel="noopener">ntfy.sh</a> (ou une instance auto-hébergée) — pas besoin de bot, juste un nom de "topic" à vous. Installez l'appli ntfy et abonnez-vous au même topic pour recevoir les alertes sur votre téléphone.
      </p>
      <div class="form-grid">
        <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="nt-enabled" style="width:auto" ${ntfyCfg.enabled ? "checked" : ""}> Activer les notifications</label></div>
        <div class="form-row"><label>Serveur ntfy</label><input id="nt-server" placeholder="https://ntfy.sh" value="${escapeHtml(ntfyCfg.server_url)}"></div>
        <div class="form-row"><label>Topic</label><input id="nt-topic" placeholder="mon-mcmanager-abc123" value="${escapeHtml(ntfyCfg.topic)}"></div>
        <div class="form-row"><label>Jeton d'authentification ${ntfyCfg.has_token ? '<span class="meta">(configuré)</span>' : '<span class="meta">(optionnel, pour un serveur protégé)</span>'}</label><input id="nt-token" type="password" placeholder="${ntfyCfg.has_token ? "Laisser vide pour conserver" : "Laisser vide si non protégé"}"></div>
      </div>
      <div class="form-grid" style="margin-top:6px">
        <label style="display:flex;align-items:center;gap:8px"><input type="checkbox" id="nt-crash" style="width:auto" ${ntfyCfg.notify_crash ? "checked" : ""}> Crash</label>
        <label style="display:flex;align-items:center;gap:8px"><input type="checkbox" id="nt-backup" style="width:auto" ${ntfyCfg.notify_backup ? "checked" : ""}> Sauvegarde terminée</label>
        <label style="display:flex;align-items:center;gap:8px"><input type="checkbox" id="nt-restart" style="width:auto" ${ntfyCfg.notify_scheduled_restart ? "checked" : ""}> Redémarrage programmé</label>
        <label style="display:flex;align-items:center;gap:8px"><input type="checkbox" id="nt-autostop" style="width:auto" ${ntfyCfg.notify_auto_stop ? "checked" : ""}> Arrêt automatique</label>
        <label style="display:flex;align-items:center;gap:8px"><input type="checkbox" id="nt-players" style="width:auto" ${ntfyCfg.notify_player_join_leave ? "checked" : ""}> Connexion/déconnexion joueur</label>
      </div>
      <div style="margin-top:10px;display:flex;gap:8px">
        <button class="btn-green" id="nt-save">${t('common.save')}</button>
        <button class="btn-ghost" id="nt-test">📨 Envoyer un test</button>
      </div>
    </div>
    <div class="card">
      <h2>Général</h2>
      <div class="form-grid">
        <div class="form-row"><label>Chemin de l'exécutable Java</label><input id="st-java" value="${escapeHtml(cfg.java_path)}"></div>
        <div class="form-row"><label>Dépôt GitHub pour les mises à jour (owner/repo)</label><input id="st-repo" value="${escapeHtml(cfg.update_repo)}"></div>
        <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="st-autocheck" style="width:auto" ${cfg.check_updates_on_start ? "checked" : ""}> Vérifier les mises à jour au démarrage</label></div>
      </div>
      <button class="btn-green" id="st-save">${t('common.save')}</button>
    </div>
    ${s ? `
    <div class="card">
      <h2>Serveur actif : ${escapeHtml(s.name)}</h2>
      <div class="form-grid">
        <div class="form-row"><label>Nom</label><input id="ss-name" value="${escapeHtml(s.name)}"></div>
        <div class="form-row"><label>RAM min (Mo)</label><input id="ss-xms" type="number" value="${s.xms_mb}"></div>
        <div class="form-row"><label>RAM max (Mo)</label><input id="ss-xmx" type="number" value="${s.xmx_mb}"></div>
        <div class="form-row"><label>Port</label><input id="ss-port" type="number" value="${s.port}"></div>
        <div class="form-row"><label>Sauvegarde auto (minutes, 0 = désactivé)</label><input id="ss-autobk" type="number" value="${s.auto_backup_minutes || 0}"></div>
        <div class="form-row"><label>Sauvegardes à conserver (vide = illimité)</label><input id="ss-bkretention" type="number" min="1" value="${s.backup_retention ?? ""}"></div>
        <div class="form-row"><label>Exécutable Java pour ce serveur</label>
          <div style="display:flex;gap:8px">
            <input id="ss-java" style="flex:1" value="${escapeHtml(s.java_path || "")}" placeholder="java (celui du système)">
            <button class="btn-ghost" id="ss-java-test" type="button">🧪 Tester</button>
          </div>
          <div id="ss-java-result" style="font-size:12px;margin-top:4px"></div>
        </div>
        <div class="form-row"><label>Arguments JVM additionnels</label><input id="ss-extraargs" value="${escapeHtml((s.extra_args || []).join(' '))}"></div>
        <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="ss-aikar" style="width:auto" ${s.aikar_flags ? "checked" : ""}> Flags de performance (Aikar)</label></div>
        <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="ss-autorestart" style="width:auto" ${s.auto_restart ? "checked" : ""}> Redémarrage automatique en cas de crash</label></div>
        <div class="form-row"><label>Délai avant redémarrage auto (secondes)</label><input id="ss-restartdelay" type="number" min="0" value="${s.auto_restart_delay_secs ?? 5}"></div>
        <div class="form-row"><label>Redémarrage programmé (minutes, 0 = désactivé)</label><input id="ss-schedrestart" type="number" min="0" value="${s.scheduled_restart_minutes || 0}"></div>
        <div class="form-row">
          <label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="ss-stopempty" style="width:auto" ${s.stop_when_empty_minutes ? "checked" : ""}> Couper le serveur si personne ne rejoint</label>
        </div>
        <div class="form-row"><label>… après combien de minutes sans joueur</label><input id="ss-stopempty-min" type="number" min="1" value="${s.stop_when_empty_minutes || 20}" ${s.stop_when_empty_minutes ? "" : "disabled"}></div>
        <div class="form-row">
          <label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="ss-dynamic" style="width:auto" ${s.dynamic_server ? "checked" : ""} ${s.stop_when_empty_minutes ? "" : "disabled"}> ⚡ Serveur dynamique (économie d'énergie)</label>
          <p style="color:var(--overlay0);font-size:12px;margin-top:4px">
            Une fois arrêté par inactivité, MCManager écoute discrètement sur le port du serveur (Java, et Bedrock/Geyser si détecté — expérimental) au lieu de laisser le port mort.
            Il répond aux pings (le serveur reste visible avec un message "en veille") et redémarre le serveur dès qu'un joueur essaie vraiment de rejoindre.
            <b>Compromis</b> : la toute première connexion après une mise en veille prend le temps normal de démarrage du serveur (pas instantané) — le joueur doit réessayer quelques secondes après le premier message de refus.
            Pour réduire ce délai, des mods comme <a href="https://modrinth.com/mod/lazydfu" target="_blank" rel="noopener">LazyDFU</a> (Fabric/Forge, accélère le démarrage en différant l'initialisation du DataFixerUpper) ou <a href="https://modrinth.com/plugin/starlight" target="_blank" rel="noopener">Starlight</a> (moteur de lumière plus rapide, Paper/Fabric) peuvent aider.
            Nécessite "Couper le serveur si personne ne rejoint" activé ci-dessus, qui décide quand la mise en veille se déclenche.
          </p>
        </div>
      </div>
      <button class="btn-green" id="ss-save">${t('common.save')}</button>
      <p style="color:var(--overlay0);font-size:12px;margin-top:8px">RAM/port/args JVM/flags Aikar s'appliquent au prochain démarrage. Le redémarrage programmé, le délai de redémarrage auto, la rétention des sauvegardes et l'arrêt sur inactivité prennent effet immédiatement, même sans redémarrer manuellement.</p>
    </div>` : ""}
    <div class="card">
      <h2>À propos</h2>
      <div style="display:flex;align-items:center;gap:12px">
        <img src="assets/icon-256.png" alt="MCManager" style="width:48px;height:48px;border-radius:10px">
        <p style="font-size:13px;color:var(--subtext1)">MCManager v<span id="about-version"></span> — Développé par Yolezz. Licence MIT.</p>
      </div>
    </div>
  `;
  $("#about-version").textContent = window.APP_VERSION || "1.0.0";

  $("#st-lang").addEventListener("change", (e) => setLang(e.target.value));

  $("#ap-fontsize").addEventListener("input", (e) => {
    setConsoleAppearance({ fontSize: parseFloat(e.target.value) });
    e.target.previousElementSibling?.remove?.(); // no-op guard; label text updated below
    const label = e.target.closest(".form-row").querySelector("label");
    if (label) label.textContent = `Taille de police (${e.target.value}px)`;
  });
  $("#ap-fontfamily").addEventListener("change", (e) => setConsoleAppearance({ fontFamily: e.target.value }));

  $("#nt-save").addEventListener("click", async () => {
    try {
      await api("/ntfy/config", { method: "POST", body: JSON.stringify({
        enabled: $("#nt-enabled").checked,
        server_url: $("#nt-server").value,
        topic: $("#nt-topic").value,
        auth_token: $("#nt-token").value,
        notify_crash: $("#nt-crash").checked,
        notify_backup: $("#nt-backup").checked,
        notify_scheduled_restart: $("#nt-restart").checked,
        notify_auto_stop: $("#nt-autostop").checked,
        notify_player_join_leave: $("#nt-players").checked,
      }) });
      toast("Configuration des notifications enregistrée.", "success");
      renderSettings();
    } catch (e) {
      toast(e.message || "Échec de l'enregistrement.", "error");
    }
  });
  $("#nt-test").addEventListener("click", async () => {
    try {
      await api("/ntfy/test", { method: "POST" });
      toast("Notification de test envoyée.", "success");
    } catch (e) {
      toast(e.message || "Échec de l'envoi (enregistrez la config d'abord).", "error");
    }
  });

  if (s) {
    $("#ss-stopempty").addEventListener("change", (e) => {
      $("#ss-stopempty-min").disabled = !e.target.checked;
      $("#ss-dynamic").disabled = !e.target.checked;
      if (!e.target.checked) $("#ss-dynamic").checked = false;
    });
    $("#ss-java-test").addEventListener("click", async () => {
      const resultEl = $("#ss-java-result");
      resultEl.textContent = "Test en cours…";
      resultEl.style.color = "var(--overlay0)";
      try {
        const xmx = parseInt($("#ss-xmx").value, 10) || s.xmx_mb;
        const javaPath = $("#ss-java").value.trim() || undefined;
        const res = await api(`/servers/${s.id}/java/test`, { method: "POST", body: JSON.stringify({ java_path: javaPath, xmx_mb: xmx }) });
        resultEl.style.color = res.ok ? "var(--green)" : "var(--red)";
        resultEl.textContent = res.ok
          ? `✓ "${res.java_path}" fonctionne avec -Xmx${res.xmx_mb}M — ${res.output.split("\n")[0]}`
          : `✗ Échec avec -Xmx${res.xmx_mb}M : ${res.output || "aucune sortie"}`;
      } catch (e) {
        resultEl.style.color = "var(--red)";
        resultEl.textContent = `✗ ${e.message || "Impossible de lancer ce Java (chemin introuvable ?)."}`;
      }
    });

    $("#ss-save").addEventListener("click", async () => {
      const extraArgs = $("#ss-extraargs").value.trim();
      const schedRestart = parseInt($("#ss-schedrestart").value, 10) || 0;
      const retention = parseInt($("#ss-bkretention").value, 10);
      const body = {
        name: $("#ss-name").value,
        xms_mb: parseInt($("#ss-xms").value, 10),
        xmx_mb: parseInt($("#ss-xmx").value, 10),
        port: parseInt($("#ss-port").value, 10),
        auto_backup_minutes: parseInt($("#ss-autobk").value, 10) || 0,
        backup_retention: retention > 0 ? retention : null,
        extra_args: extraArgs ? extraArgs.split(/\s+/) : [],
        aikar_flags: $("#ss-aikar").checked,
        auto_restart: $("#ss-autorestart").checked,
        auto_restart_delay_secs: parseInt($("#ss-restartdelay").value, 10) || 0,
        scheduled_restart_minutes: schedRestart > 0 ? schedRestart : null,
        stop_when_empty_minutes: $("#ss-stopempty").checked ? (parseInt($("#ss-stopempty-min").value, 10) || 20) : null,
        dynamic_server: $("#ss-stopempty").checked && $("#ss-dynamic").checked,
        java_path: $("#ss-java").value.trim() || null,
      };
      // The PUT response IS the freshly-saved server entry - apply it
      // straight into state.servers instead of just tossing it, otherwise
      // the very next renderSettings() call reads currentServer() from the
      // stale pre-save cache and the form appears to "not have saved"
      // even though the backend wrote the change correctly.
      const updated = await api(`/servers/${s.id}`, { method: "PUT", body: JSON.stringify(body) });
      state.servers = state.servers.map((sv) => sv.id === updated.id ? updated : sv);
      toast("Paramètres du serveur enregistrés.", "success");
      renderSettings();
    });
  }

  $("#st-save").addEventListener("click", async () => {
    const newCfg = {
      ...cfg,
      java_path: $("#st-java").value,
      update_repo: $("#st-repo").value,
      check_updates_on_start: $("#st-autocheck").checked,
    };
    await api("/settings", { method: "PUT", body: JSON.stringify(newCfg) });
    toast("Paramètres enregistrés.", "success");
  });
}

// ───────────────────────── update banner ─────────────────────────

async function checkUpdateBanner() {
  try {
    const info = await api("/update/check");
    window.APP_VERSION = info.current_version;
    $("#app-version").textContent = "v" + info.current_version;
    if (info.update_available) {
      $("#update-banner").classList.remove("hidden");
      if (info.self_update_supported) {
        $("#update-text").textContent = `Mise à jour ${info.latest_version} disponible !`;
        $("#update-apply-btn").classList.remove("hidden");
        $("#update-apply-btn").onclick = async () => {
          if (!confirm("Télécharger et installer la mise à jour ? Redémarrez MCManager ensuite.")) return;
          await api("/update/apply", { method: "POST" });
          toast("Mise à jour installée. Redémarrez MCManager.", "success");
        };
      } else {
        $("#update-text").textContent = `Mise à jour ${info.latest_version} disponible — ${info.self_update_note || "mise à jour manuelle requise pour ce type d'installation."}`;
        $("#update-apply-btn").classList.add("hidden");
      }
    }
  } catch {}
}

// ───────────────────────── boot ─────────────────────────

applyConsoleAppearance();
setupNav();
refreshServerList().then(render);
checkUpdateBanner();
setInterval(() => { if (state.view === "dashboard") renderDashboard(); }, 10000);
