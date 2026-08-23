// MCManager — internationalisation (FR / EN / ES).
//
// Kept deliberately simple (flat key → string per language, `{placeholders}`
// substitution, no build step) to match the rest of the frontend: plain JS
// served as static files by the Rust binary, no bundler.
//
// Coverage note: this first pass covers the app chrome (navigation, common
// buttons, view titles/subtitles, settings) — the highest-traffic text.
// Long-form copy (the docs/tutorial pages, the server-creation wizard's
// helper text) is still French-only pending a follow-up pass; it falls back
// to French automatically (see `t()` below) rather than showing a raw key.

const TRANSLATIONS = {
  fr: {
    "nav.dashboard": "📊 Tableau de bord",
    "nav.servers": "🖥 Serveurs",
    "nav.console": "📟 Console",
    "nav.files": "📁 Fichiers",
    "nav.addons": "🧩 Mods / Plugins",
    "nav.marketplace": "🛒 Marketplace",
    "nav.schematics": "🏗 Schematics",
    "nav.whitelist": "🛡 Liste blanche",
    "nav.properties": "📝 Propriétés serveur",
    "nav.backups": "💾 Sauvegardes",
    "nav.stats": "📈 Statistiques",
    "nav.network": "🌐 Réseau (playit.gg)",
    "nav.docs": "📚 Docs & tutos",
    "nav.assistant": "🤖 Assistant IA",
    "nav.settings": "⚙ Paramètres",
    "nav.activeServer": "Serveur actif",
    "nav.none": "— aucun —",

    "common.delete": "Supprimer",
    "common.cancel": "Annuler",
    "common.save": "Enregistrer",
    "common.loading": "Chargement…",
    "common.language": "Langue",

    "addons.config": "⚙ Config",
    "addons.configTip": "Ouvrir le dossier de ce mod/plugin dans l'explorateur de fichiers",
    "addons.enable": "Activer",
    "addons.disable": "Désactiver",

    "files.openExplorer": "Ouvrir dans l'explorateur",
    "files.explorerFailed": "Impossible d'ouvrir l'explorateur de fichiers (pas d'environnement de bureau sur cette machine ?).",

    "settings.language": "Langue de l'interface",

    "docs.title": "Documentation &amp; tutoriels",
    "docs.s1_h": "Créer un serveur",
    "docs.s1_p": "Allez dans <b>Serveurs → Nouveau serveur</b>, choisissez un type (Paper est recommandé pour les plugins, Fabric pour les mods légers), une version, acceptez l'EULA et cliquez sur Créer. MCManager télécharge et configure tout automatiquement.",
    "docs.s2_h": "Installer des mods/plugins",
    "docs.s2_p": "Depuis l'onglet <b>Marketplace</b>, recherchez un mod/plugin et cliquez sur Installer — MCManager choisit automatiquement la bonne version pour votre loader et votre version de Minecraft. Redémarrez le serveur pour l'activer.",
    "docs.s3_h": "WorldEdit / FastAsyncWorldEdit",
    "docs.s3_p": "Installez WorldEdit ou FAWE depuis la recherche du <b>Marketplace</b> (onglet Mods/Plugins), puis déposez vos fichiers <code>.schem</code> depuis l'onglet <b>Schematics</b>. Chargez-les en jeu avec <code>//schem load nom_du_fichier</code> puis <code>//paste</code>.",
    "docs.s4_h": "Rendre le serveur accessible depuis Internet",
    "docs.s4_p": "Deux options : ouvrez le port du serveur (par défaut 25565) sur votre routeur (redirection de port / NAT), ou utilisez <b>Réseau → playit.gg</b> qui ne nécessite aucune configuration réseau.",
    "docs.s5_h": "Sauvegardes automatiques",
    "docs.s5_p": "Réglez un intervalle de sauvegarde automatique par serveur depuis les Paramètres du serveur — une sauvegarde .zip est créée automatiquement pendant que le serveur tourne.",
    "docs.s6_h": "Mise à jour de MCManager",
    "docs.s6_p": "MCManager vérifie automatiquement les nouvelles versions au démarrage (releases GitHub). Une bannière apparaît si une mise à jour est disponible ; cliquez sur Mettre à jour pour l'appliquer, puis redémarrez l'application.",
  },
  en: {
    "nav.dashboard": "📊 Dashboard",
    "nav.servers": "🖥 Servers",
    "nav.console": "📟 Console",
    "nav.files": "📁 Files",
    "nav.addons": "🧩 Mods / Plugins",
    "nav.marketplace": "🛒 Marketplace",
    "nav.schematics": "🏗 Schematics",
    "nav.whitelist": "🛡 Whitelist",
    "nav.properties": "📝 Server properties",
    "nav.backups": "💾 Backups",
    "nav.stats": "📈 Statistics",
    "nav.network": "🌐 Network (playit.gg)",
    "nav.docs": "📚 Docs & tutorials",
    "nav.assistant": "🤖 AI assistant",
    "nav.settings": "⚙ Settings",
    "nav.activeServer": "Active server",
    "nav.none": "— none —",

    "common.delete": "Delete",
    "common.cancel": "Cancel",
    "common.save": "Save",
    "common.loading": "Loading…",
    "common.language": "Language",

    "addons.config": "⚙ Config",
    "addons.configTip": "Open this mod/plugin's folder in the file browser",
    "addons.enable": "Enable",
    "addons.disable": "Disable",

    "files.openExplorer": "Open in file explorer",
    "files.explorerFailed": "Couldn't open the file explorer (no desktop environment on this machine?).",

    "settings.language": "Interface language",

    "docs.title": "Documentation &amp; tutorials",
    "docs.s1_h": "Create a server",
    "docs.s1_p": "Go to <b>Servers → New server</b>, pick a type (Paper is recommended for plugins, Fabric for lightweight mods), a version, accept the EULA and click Create. MCManager downloads and configures everything automatically.",
    "docs.s2_h": "Install mods/plugins",
    "docs.s2_p": "From the <b>Marketplace</b> tab, search for a mod/plugin and click Install — MCManager automatically picks the right version for your loader and your Minecraft version. Restart the server to activate it.",
    "docs.s3_h": "WorldEdit / FastAsyncWorldEdit",
    "docs.s3_p": "Install WorldEdit or FAWE from the <b>Marketplace</b> search (Mods/Plugins tab), then drop your <code>.schem</code> files from the <b>Schematics</b> tab. Load them in-game with <code>//schem load file_name</code> then <code>//paste</code>.",
    "docs.s4_h": "Making the server reachable from the Internet",
    "docs.s4_p": "Two options: open the server port (25565 by default) on your router (port forwarding / NAT), or use <b>Network → playit.gg</b>, which needs no network configuration at all.",
    "docs.s5_h": "Automatic backups",
    "docs.s5_p": "Set an automatic backup interval per server from the server's Settings — a .zip backup is created automatically while the server is running.",
    "docs.s6_h": "Updating MCManager",
    "docs.s6_p": "MCManager automatically checks for new versions on startup (GitHub releases). A banner appears if an update is available; click Update to apply it, then restart the app.",
  },
  es: {
    "nav.dashboard": "📊 Panel",
    "nav.servers": "🖥 Servidores",
    "nav.console": "📟 Consola",
    "nav.files": "📁 Archivos",
    "nav.addons": "🧩 Mods / Plugins",
    "nav.marketplace": "🛒 Mercado",
    "nav.schematics": "🏗 Schematics",
    "nav.whitelist": "🛡 Lista blanca",
    "nav.properties": "📝 Propiedades del servidor",
    "nav.backups": "💾 Copias de seguridad",
    "nav.stats": "📈 Estadísticas",
    "nav.network": "🌐 Red (playit.gg)",
    "nav.docs": "📚 Docs y tutoriales",
    "nav.settings": "⚙ Ajustes",
    "nav.assistant": "🤖 Asistente IA",
    "nav.activeServer": "Servidor activo",
    "nav.none": "— ninguno —",

    "common.delete": "Eliminar",
    "common.cancel": "Cancelar",
    "common.save": "Guardar",
    "common.loading": "Cargando…",
    "common.language": "Idioma",

    "addons.config": "⚙ Config",
    "addons.configTip": "Abrir la carpeta de este mod/plugin en el explorador de archivos",
    "addons.enable": "Activar",
    "addons.disable": "Desactivar",

    "files.openExplorer": "Abrir en el explorador de archivos",
    "files.explorerFailed": "No se pudo abrir el explorador de archivos (¿sin entorno de escritorio en esta máquina?).",

    "settings.language": "Idioma de la interfaz",

    "docs.title": "Documentación y tutoriales",
    "docs.s1_h": "Crear un servidor",
    "docs.s1_p": "Ve a <b>Servidores → Nuevo servidor</b>, elige un tipo (Paper es recomendado para plugins, Fabric para mods ligeros), una versión, acepta el EULA y haz clic en Crear. MCManager descarga y configura todo automáticamente.",
    "docs.s2_h": "Instalar mods/plugins",
    "docs.s2_p": "Desde la pestaña <b>Mercado</b>, busca un mod/plugin y haz clic en Instalar — MCManager elige automáticamente la versión correcta para tu loader y tu versión de Minecraft. Reinicia el servidor para activarlo.",
    "docs.s3_h": "WorldEdit / FastAsyncWorldEdit",
    "docs.s3_p": "Instala WorldEdit o FAWE desde la búsqueda del <b>Mercado</b> (pestaña Mods/Plugins), luego coloca tus archivos <code>.schem</code> desde la pestaña <b>Schematics</b>. Cárgalos en el juego con <code>//schem load nombre_archivo</code> y luego <code>//paste</code>.",
    "docs.s4_h": "Hacer el servidor accesible desde Internet",
    "docs.s4_p": "Dos opciones: abre el puerto del servidor (25565 por defecto) en tu router (redirección de puertos / NAT), o usa <b>Red → playit.gg</b>, que no necesita ninguna configuración de red.",
    "docs.s5_h": "Copias de seguridad automáticas",
    "docs.s5_p": "Configura un intervalo de copia de seguridad automática por servidor desde los Ajustes del servidor — se crea un .zip automáticamente mientras el servidor está en marcha.",
    "docs.s6_h": "Actualizar MCManager",
    "docs.s6_p": "MCManager verifica automáticamente las nuevas versiones al iniciar (releases de GitHub). Aparece un banner si hay una actualización disponible; haz clic en Actualizar para aplicarla, luego reinicia la aplicación.",
  },
};

const SUPPORTED_LANGS = ["fr", "en", "es"];

function detectDefaultLang() {
  const stored = localStorage.getItem("mcmanager-lang");
  if (stored && SUPPORTED_LANGS.includes(stored)) return stored;
  const nav = (navigator.language || "fr").slice(0, 2).toLowerCase();
  return SUPPORTED_LANGS.includes(nav) ? nav : "fr";
}

state.lang = detectDefaultLang();

function setLang(lang) {
  if (!SUPPORTED_LANGS.includes(lang)) return;
  state.lang = lang;
  localStorage.setItem("mcmanager-lang", lang);
  document.documentElement.lang = lang;
  applyStaticTranslations();
  render();
}

/// Translates `key`, falling back to French then to the raw key so a missing
/// translation shows readable (French) text instead of "undefined" or a key
/// slug — the app stays usable while a language's coverage is incomplete.
function t(key, vars) {
  let str = TRANSLATIONS[state.lang]?.[key] ?? TRANSLATIONS.fr[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) str = str.replaceAll(`{${k}}`, v);
  }
  return str;
}

// Translates elements present in the static index.html (nav items, sidebar
// labels) that exist before the SPA's own render() ever runs.
function applyStaticTranslations() {
  document.documentElement.lang = state.lang;
  $$("[data-view]").forEach((btn) => {
    const key = `nav.${btn.dataset.view}`;
    if (TRANSLATIONS.fr[key]) btn.textContent = t(key);
  });
  const pickerLabel = $(".server-picker label");
  if (pickerLabel) pickerLabel.textContent = t("nav.activeServer");
  const noneOpt = $("#server-select option[value='']");
  if (noneOpt) noneOpt.textContent = t("nav.none");
}

applyStaticTranslations();
