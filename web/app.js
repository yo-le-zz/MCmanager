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
      case "backups": return renderBackups();
      case "network": return renderNetwork();
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
    <h1>Tableau de bord</h1>
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
    <h1>Serveurs</h1>
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
          <button class="btn-red" data-del="${s.id}">Supprimer</button>
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
    <h1>Console — ${escapeHtml(s.name)}</h1>
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
    <div class="console-input-row">
      <input id="console-cmd" placeholder="Tapez une commande serveur (ex: say bonjour) puis Entrée">
      <button class="btn-blue" id="console-send">Envoyer</button>
    </div>
  `;

  const out = $("#console-out");
  function appendLine(line) {
    const atBottom = out.scrollTop + out.clientHeight >= out.scrollHeight - 30;
    out.textContent += line + "\n";
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

  function send() {
    const input = $("#console-cmd");
    const cmd = input.value.trim();
    if (!cmd) return;
    if (DANGEROUS_COMMANDS.includes(cmd.toLowerCase())) {
      if (!confirm(`Cette commande ("${cmd}") va arrêter ou recharger le serveur. Confirmer ?`)) return;
    }
    ws.readyState === 1 ? ws.send(cmd) : api(`/servers/${s.id}/command`, { method: "POST", body: JSON.stringify({ cmd }) });
    input.value = "";
  }
  $("#console-send").addEventListener("click", send);
  $("#console-cmd").addEventListener("keydown", (e) => { if (e.key === "Enter") send(); });

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
    <h1>Fichiers — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Chemin actuel : /${escapeHtml(path)}</div>
    <div class="toolbar">
      ${path ? '<button class="btn-ghost" id="f-up">⬆ Dossier parent</button>' : ""}
      <label class="btn-ghost" style="cursor:pointer">📤 Envoyer un fichier<input type="file" id="f-upload" class="hidden"></label>
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
    <h1>${isModded ? "Mods" : "Plugins"} — ${escapeHtml(s.name)}</h1>
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
  const modList = $("#mod-list");
  modList.innerHTML = addons.length ? addons.map((a) => `
    <div class="mod-row">
      <div>
        <div class="name">${escapeHtml(a.file_name)}</div>
        <div class="meta">${humanSize(a.size_bytes)} · ${a.enabled ? "Activé" : "Désactivé"}</div>
      </div>
      <div class="mod-actions">
        <button class="btn-ghost" data-config="${escapeHtml(a.file_name)}" title="${t('addons.configTip')}">${t('addons.config')}</button>
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
    <h1>Marketplace</h1>
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
        <button class="btn-red" data-del-sc="${escapeHtml(f.name)}">Supprimer</button>
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
    <h1>Sauvegardes — ${escapeHtml(s.name)}</h1>
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
        <button class="btn-red" data-delbk="${escapeHtml(b.name)}">Supprimer</button>
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

// ───────────────────────── network (playit.gg) ─────────────────────────

async function renderNetwork() {
  const content = $("#content");
  const status = await api("/playit/status");
  content.innerHTML = `
    <h1>Réseau — playit.gg</h1>
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
    cfg = { provider: "anthropic", model: "", ollama_base_url: "", has_key: false, masked_key: "" };
  }

  content.innerHTML = `
    <h1>🤖 Assistant IA</h1>
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
        <div class="form-row">
          <label>Modèle</label>
          <div style="display:flex;gap:8px">
            <select id="ai-model" style="flex:1"><option value="${escapeHtml(cfg.model || "")}">${escapeHtml(cfg.model || "(par défaut)")}</option></select>
            <button class="btn-ghost" id="ai-load-models" type="button">🔄 Détecter les modèles</button>
          </div>
        </div>
      </div>
      <button class="btn-green" id="ai-save">Enregistrer</button>
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
    $("#ai-key-row").style.display = e.target.value === "ollama" ? "none" : "";
  });

  // Auto-detect provider from the shape of a pasted key (sk-ant-... /
  // sk-... / AIza...), mirroring the same heuristic the backend uses.
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

function renderAiChatLog() {
  const log = $("#ai-chat-log");
  if (!log) return;
  log.innerHTML = state.aiChatHistory.length
    ? state.aiChatHistory.map((m) => `<div style="margin-bottom:10px"><b>${m.role === "user" ? "Vous" : "Assistant"} :</b> ${escapeHtml(m.content).replace(/\n/g, "<br>")}</div>`).join("")
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
  content.innerHTML = `
    <h1>Paramètres</h1>
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
      <h2>Général</h2>
      <div class="form-grid">
        <div class="form-row"><label>Chemin de l'exécutable Java</label><input id="st-java" value="${escapeHtml(cfg.java_path)}"></div>
        <div class="form-row"><label>Dépôt GitHub pour les mises à jour (owner/repo)</label><input id="st-repo" value="${escapeHtml(cfg.update_repo)}"></div>
        <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="st-autocheck" style="width:auto" ${cfg.check_updates_on_start ? "checked" : ""}> Vérifier les mises à jour au démarrage</label></div>
      </div>
      <button class="btn-green" id="st-save">Enregistrer</button>
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
        <div class="form-row"><label>Arguments JVM additionnels</label><input id="ss-extraargs" value="${escapeHtml((s.extra_args || []).join(' '))}"></div>
        <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="ss-aikar" style="width:auto" ${s.aikar_flags ? "checked" : ""}> Flags de performance (Aikar)</label></div>
        <div class="form-row"><label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="ss-autorestart" style="width:auto" ${s.auto_restart ? "checked" : ""}> Redémarrage automatique en cas de crash</label></div>
        <div class="form-row"><label>Délai avant redémarrage auto (secondes)</label><input id="ss-restartdelay" type="number" min="0" value="${s.auto_restart_delay_secs ?? 5}"></div>
        <div class="form-row"><label>Redémarrage programmé (minutes, 0 = désactivé)</label><input id="ss-schedrestart" type="number" min="0" value="${s.scheduled_restart_minutes || 0}"></div>
        <div class="form-row">
          <label style="display:flex;align-items:center;gap:8px;margin-top:20px"><input type="checkbox" id="ss-stopempty" style="width:auto" ${s.stop_when_empty_minutes ? "checked" : ""}> Couper le serveur si personne ne rejoint</label>
        </div>
        <div class="form-row"><label>… après combien de minutes sans joueur</label><input id="ss-stopempty-min" type="number" min="1" value="${s.stop_when_empty_minutes || 20}" ${s.stop_when_empty_minutes ? "" : "disabled"}></div>
      </div>
      <button class="btn-green" id="ss-save">Enregistrer</button>
      <p style="color:var(--overlay0);font-size:12px;margin-top:8px">RAM/port/args JVM/flags Aikar s'appliquent au prochain démarrage. Le redémarrage programmé, le délai de redémarrage auto et l'arrêt sur inactivité prennent effet immédiatement, même sans redémarrer manuellement.</p>
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

  if (s) {
    $("#ss-stopempty").addEventListener("change", (e) => {
      $("#ss-stopempty-min").disabled = !e.target.checked;
    });
    $("#ss-save").addEventListener("click", async () => {
      const extraArgs = $("#ss-extraargs").value.trim();
      const schedRestart = parseInt($("#ss-schedrestart").value, 10) || 0;
      const body = {
        name: $("#ss-name").value,
        xms_mb: parseInt($("#ss-xms").value, 10),
        xmx_mb: parseInt($("#ss-xmx").value, 10),
        port: parseInt($("#ss-port").value, 10),
        auto_backup_minutes: parseInt($("#ss-autobk").value, 10) || 0,
        extra_args: extraArgs ? extraArgs.split(/\s+/) : [],
        aikar_flags: $("#ss-aikar").checked,
        auto_restart: $("#ss-autorestart").checked,
        auto_restart_delay_secs: parseInt($("#ss-restartdelay").value, 10) || 0,
        scheduled_restart_minutes: schedRestart > 0 ? schedRestart : null,
        stop_when_empty_minutes: $("#ss-stopempty").checked ? (parseInt($("#ss-stopempty-min").value, 10) || 20) : null,
      };
      await api(`/servers/${s.id}`, { method: "PUT", body: JSON.stringify(body) });
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

setupNav();
refreshServerList().then(render);
checkUpdateBanner();
setInterval(() => { if (state.view === "dashboard") renderDashboard(); }, 10000);
