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
  sel.innerHTML = '<option value="">— aucun —</option>' +
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
    </div>
    <div id="server-list" class="grid"></div>
    <div id="wizard" class="card hidden"></div>
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
    $$("[data-del]").forEach((b) => b.addEventListener("click", async () => {
      if (!confirm("Supprimer définitivement ce serveur et tous ses fichiers ?")) return;
      await api(`/servers/${b.dataset.del}`, { method: "DELETE" });
      toast("Serveur supprimé.", "success");
      renderServers();
    }));
  }

  $("#new-server-btn").addEventListener("click", () => openWizard());
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
          <option value="forge">Forge (mods)</option>
          <option value="neoforge">NeoForge (mods, successeur de Forge)</option>
        </select>
      </div>
      <div class="form-row">
        <label>Version de Minecraft</label>
        <select id="w-version"><option>Chargement…</option></select>
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
    <button class="btn-green" id="w-create">Créer le serveur</button>
    <span id="w-status" style="margin-left:12px;color:var(--subtext1)"></span>
  `;

  const loaderSel = $("#w-loader");
  const versionSel = $("#w-version");
  async function loadVersions() {
    versionSel.innerHTML = "<option>Chargement…</option>";
    const versions = await api(`/loaders/${loaderSel.value}/versions`);
    versionSel.innerHTML = versions.map((v) => `<option value="${v}">${v}</option>`).join("");
  }
  loaderSel.addEventListener("change", loadVersions);
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
      const body = {
        name: $("#w-name").value || "Nouveau serveur",
        loader: loaderSel.value,
        mc_version: versionSel.value,
        xms_mb: parseInt($("#w-xms").value, 10),
        xmx_mb: parseInt($("#w-xmx").value, 10),
        port: parseInt($("#w-port").value, 10),
        eula_accepted: true,
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

  function send() {
    const input = $("#console-cmd");
    const cmd = input.value.trim();
    if (!cmd) return;
    ws.readyState === 1 ? ws.send(cmd) : api(`/servers/${s.id}/command`, { method: "POST", body: JSON.stringify({ cmd }) });
    input.value = "";
  }
  $("#console-send").addEventListener("click", send);
  $("#console-cmd").addEventListener("keydown", (e) => { if (e.key === "Enter") send(); });

  $("#c-start").addEventListener("click", async () => { await api(`/servers/${s.id}/start`, { method: "POST" }); toast("Démarrage…", "success"); });
  $("#c-stop").addEventListener("click", async () => { await api(`/servers/${s.id}/stop`, { method: "POST" }); toast("Arrêt demandé.", "success"); });
  $("#c-kill").addEventListener("click", async () => { if (confirm("Forcer l'arrêt immédiat ?")) { await api(`/servers/${s.id}/kill`, { method: "POST" }); } });
  $("#c-restart").addEventListener("click", async () => {
    try { await api(`/servers/${s.id}/stop`, { method: "POST" }); } catch {}
    setTimeout(async () => { try { await api(`/servers/${s.id}/start`, { method: "POST" }); toast("Redémarrage…", "success"); } catch {} }, 4000);
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
  const isModded = ["fabric", "forge", "neoforge"].includes(s.loader);
  content.innerHTML = `
    <h1>${isModded ? "Mods" : "Plugins"} — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Activez, désactivez ou supprimez vos ${isModded ? "mods" : "plugins"} installés.</div>
    <div class="toolbar">
      <button class="btn-blue" id="check-updates">🔄 Vérifier les mises à jour</button>
      <span id="updates-result"></span>
    </div>
    <div class="card">
      <h2>Préréglages recommandés</h2>
      <div id="presets" class="preset-grid"><div class="empty-state">Chargement…</div></div>
    </div>
    <div class="card">
      <h2>Installés (${addons.length})</h2>
      <div class="mod-list" id="mod-list"></div>
    </div>
  `;

  const modList = $("#mod-list");
  modList.innerHTML = addons.length ? addons.map((a) => `
    <div class="mod-row">
      <div>
        <div class="name">${escapeHtml(a.file_name)}</div>
        <div class="meta">${humanSize(a.size_bytes)} · ${a.enabled ? "Activé" : "Désactivé"}</div>
      </div>
      <div class="mod-actions">
        <button class="btn-ghost" data-toggle="${escapeHtml(a.file_name)}">${a.enabled ? "Désactiver" : "Activer"}</button>
        <button class="btn-red" data-remove="${escapeHtml(a.file_name)}">Supprimer</button>
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

  const presets = await api("/presets");
  const compatible = presets.filter((p) => p.loaders.includes(s.loader));
  $("#presets").innerHTML = compatible.length ? compatible.map((p) => `
    <div class="preset-card">
      <div class="cat">${escapeHtml(p.category)}</div>
      <div class="name" style="font-weight:700">${escapeHtml(p.label)}</div>
      <div class="meta" style="margin:6px 0">${escapeHtml(p.description)}</div>
      <button class="btn-mauve" data-install-preset="${p.key}">+ Installer</button>
    </div>
  `).join("") : '<div class="empty-state">Aucun préréglage compatible avec ce type de serveur.</div>';

  $$("[data-install-preset]").forEach((b) => b.addEventListener("click", async () => {
    b.disabled = true;
    b.textContent = "Installation…";
    try {
      await api(`/servers/${s.id}/presets/${b.dataset.installPreset}/install`, { method: "POST" });
      toast("Installé avec succès.", "success");
      renderAddons();
    } catch {
      b.disabled = false;
      b.textContent = "+ Installer";
    }
  }));

  $("#check-updates").addEventListener("click", async () => {
    $("#updates-result").textContent = "Vérification en cours…";
    const updates = await api(`/servers/${s.id}/marketplace/updates`);
    $("#updates-result").textContent = updates.length ? `${updates.length} mise(s) à jour disponible(s).` : "Tout est à jour.";
  });
}

// ───────────────────────── marketplace ─────────────────────────

async function renderMarketplace() {
  const s = currentServer();
  const content = $("#content");
  const projectType = ["fabric", "forge", "neoforge"].includes(s.loader) ? "mod" : "plugin";
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

async function renderBackups() {
  const s = currentServer();
  const content = $("#content");
  const backups = await api(`/servers/${s.id}/backups`);
  content.innerHTML = `
    <h1>Sauvegardes — ${escapeHtml(s.name)}</h1>
    <div class="subtitle">Sauvegardes complètes (mondes, configs, mods/plugins) au format .zip.</div>
    <div class="toolbar">
      <button class="btn-blue" id="bk-create">💾 Créer une sauvegarde maintenant</button>
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
    toast("Création de la sauvegarde…", "success");
    await api(`/servers/${s.id}/backups`, { method: "POST" });
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
        <button class="btn-blue" id="pl-download">⬇ Installer / mettre à jour l'agent</button>
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
  $("#pl-start").addEventListener("click", async () => { await api("/playit/start", { method: "POST" }); renderNetwork(); });
  $("#pl-stop").addEventListener("click", async () => { await api("/playit/stop", { method: "POST" }); renderNetwork(); });
}

// ───────────────────────── docs ─────────────────────────

function renderDocs() {
  const content = $("#content");
  content.innerHTML = `
    <h1>Documentation &amp; tutoriels</h1>
    <div class="card doc-block">
      <h3>Créer un serveur</h3>
      <p>Allez dans <b>Serveurs → Nouveau serveur</b>, choisissez un type (Paper est recommandé pour les plugins, Fabric pour les mods légers), une version, acceptez l'EULA et cliquez sur Créer. MCManager télécharge et configure tout automatiquement.</p>
      <h3>Installer des mods/plugins</h3>
      <p>Depuis l'onglet <b>Marketplace</b>, recherchez un mod/plugin et cliquez sur Installer — MCManager choisit automatiquement la bonne version pour votre loader et votre version de Minecraft. Redémarrez le serveur pour l'activer.</p>
      <h3>WorldEdit / FastAsyncWorldEdit</h3>
      <p>Installez WorldEdit ou FAWE depuis les préréglages de l'onglet Mods/Plugins, puis déposez vos fichiers <code>.schem</code> depuis l'onglet <b>Schematics</b>. Chargez-les en jeu avec <code>//schem load nom_du_fichier</code> puis <code>//paste</code>.</p>
      <h3>Rendre le serveur accessible depuis Internet</h3>
      <p>Deux options : ouvrez le port du serveur (par défaut 25565) sur votre routeur (redirection de port / NAT), ou utilisez <b>Réseau → playit.gg</b> qui ne nécessite aucune configuration réseau.</p>
      <h3>Sauvegardes automatiques</h3>
      <p>Réglez un intervalle de sauvegarde automatique par serveur depuis les Paramètres du serveur — une sauvegarde .zip est créée automatiquement pendant que le serveur tourne.</p>
      <h3>Mise à jour de MCManager</h3>
      <p>MCManager vérifie automatiquement les nouvelles versions au démarrage (releases GitHub). Une bannière apparaît si une mise à jour est disponible ; cliquez sur Mettre à jour pour l'appliquer, puis redémarrez l'application.</p>
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
        <div class="form-row"><label>RAM min (Mo)</label><input id="ss-xms" type="number" value="${s.xms_mb}"></div>
        <div class="form-row"><label>RAM max (Mo)</label><input id="ss-xmx" type="number" value="${s.xmx_mb}"></div>
        <div class="form-row"><label>Port</label><input id="ss-port" type="number" value="${s.port}"></div>
        <div class="form-row"><label>Sauvegarde auto (minutes, 0 = désactivé)</label><input id="ss-autobk" type="number" value="${s.auto_backup_minutes || 0}"></div>
      </div>
      <p style="color:var(--overlay0);font-size:12px">Ces réglages seront appliqués au prochain démarrage du serveur. Modification directe du fichier server.properties disponible dans l'onglet Fichiers.</p>
    </div>` : ""}
    <div class="card">
      <h2>À propos</h2>
      <p style="font-size:13px;color:var(--subtext1)">MCManager v<span id="about-version"></span> — Développé par Yolezz. Licence MIT.</p>
    </div>
  `;
  $("#about-version").textContent = window.APP_VERSION || "1.0.0";

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
