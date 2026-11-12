/* MCManager — icônes, apparence personnalisable et fond animé.
 *
 * Chargé dans <head> avant tout le reste : le thème est appliqué avant le premier affichage
 * (pas de flash). Les réglages sont stockés dans le navigateur (localStorage, clé mcm-appearance).
 */
(function () {
  "use strict";
  const KEY = "mcm-appearance";
  // Collecte des erreurs JavaScript (consultable dans la console du navigateur : window.__mcmErrors).
  window.__mcmErrors = [];
  window.addEventListener("error", (e) => window.__mcmErrors.push(`${e.message} @${String(e.filename || "").split("/").pop()}:${e.lineno}`));
  window.addEventListener("unhandledrejection", (e) => window.__mcmErrors.push(`promesse rejetée : ${(e.reason && e.reason.message) || e.reason}`));
  const root = document.documentElement;

  // ---------- Icônes (sprite Lucide, assets/icons.svg) ----------
  window.ico = (name, cls = "") => `<svg class="i ${cls}" aria-hidden="true"><use href="assets/icons.svg#${name}"/></svg>`;

  // ---------- Thèmes ----------
  const THEMES = [
    ["auto", "Automatique", "Suit le thème du système", ["#0e1015", "#14171e", "#f6f7fa"]],
    ["graphite", "Graphite", "Sombre neutre", ["#0e1015", "#14171e", "#242936"]],
    ["midnight", "Minuit", "Sombre bleuté", ["#070b16", "#0d1424", "#1a2744"]],
    ["forest", "Forêt", "Sombre vert", ["#0b110e", "#111a15", "#1f3026"]],
    ["light", "Clair", "Lumineux", ["#f6f7fa", "#ffffff", "#e2e5ec"]],
    ["contrast", "Contraste élevé", "Accessibilité", ["#000000", "#0a0a0a", "#5a5a5a"]],
    ["custom", "Personnalisé", "Vos propres couleurs", null],
  ];
  const ACCENTS = [["Indigo", "#6d7cff"], ["Bleu", "#3b82f6"], ["Cyan", "#06b6d4"], ["Émeraude", "#10b981"], ["Citron", "#a3e635"], ["Ambre", "#f59e0b"], ["Orange", "#f97316"], ["Rose", "#f43f5e"], ["Violet", "#a78bfa"], ["Ardoise", "#94a3b8"]];
  const DEFAULTS = { theme: "graphite", accent: "#6d7cff", radius: 10, density: 1, font: "inter", fontScale: 1, sidebar: 248, bg: "dots", bgIntensity: 55, bgInteractive: true, bgQuality: "balanced", motion: true, custom: null, customCss: "" };

  // Les 14 jetons d'un thème : clé JSON ↔ variable CSS (doit rester alignée sur style.css).
  const TOKENS = [
    ["bg", "--bg"], ["elev", "--bg-elev"], ["sunken", "--bg-sunken"], ["hover", "--bg-hover"],
    ["border", "--border"], ["borderStrong", "--border-strong"],
    ["text", "--text"], ["dim", "--text-dim"], ["faint", "--text-faint"],
    ["ok", "--ok"], ["warn", "--warn"], ["danger", "--danger"], ["info", "--info"], ["violet", "--violet"],
  ];
  const DARK_STATUS = { ok: "#34d399", warn: "#fbbf24", danger: "#f87171", info: "#60a5fa", violet: "#a78bfa" };
  const LIGHT_STATUS = { ok: "#059669", warn: "#b45309", danger: "#dc2626", info: "#2563eb", violet: "#7c3aed" };
  // Palettes des thèmes intégrés (miroir de style.css) : servent de point de départ au thème personnalisé.
  const PALETTES = {
    graphite: { bg: "#0e1015", elev: "#14171e", sunken: "#0a0c10", hover: "#1b1f28", border: "#242936", borderStrong: "#323949", text: "#e6e9f2", dim: "#a4abbd", faint: "#6b7287", ...DARK_STATUS },
    midnight: { bg: "#070b16", elev: "#0d1424", sunken: "#050810", hover: "#142038", border: "#1a2744", borderStrong: "#27396a", text: "#e2e8f8", dim: "#9aa8cc", faint: "#5f6f9a", ...DARK_STATUS },
    forest: { bg: "#0b110e", elev: "#111a15", sunken: "#080d0a", hover: "#182419", border: "#1f3026", borderStrong: "#2e4a39", text: "#e4efe8", dim: "#9fb5a8", faint: "#65806f", ...DARK_STATUS },
    light: { bg: "#f6f7fa", elev: "#ffffff", sunken: "#eceef3", hover: "#f0f2f7", border: "#e2e5ec", borderStrong: "#cfd4de", text: "#151826", dim: "#4b5266", faint: "#8089a0", ...LIGHT_STATUS },
    contrast: { bg: "#000000", elev: "#0a0a0a", sunken: "#000000", hover: "#161616", border: "#5a5a5a", borderStrong: "#8a8a8a", text: "#ffffff", dim: "#e0e0e0", faint: "#b0b0b0", ...DARK_STATUS },
  };
  const SHADOW_DARK = "0 1px 2px rgba(0,0,0,.35), 0 8px 24px -12px rgba(0,0,0,.5)";
  const SHADOW_LIGHT = "0 1px 2px rgba(20,24,40,.06), 0 8px 24px -12px rgba(20,24,40,.18)";
  const MAX_CSS = 20000;

  // ---------- Couleurs ----------
  const HEX = /^#[0-9a-f]{6}$/i;
  const isHex = (v) => typeof v === "string" && HEX.test(v);
  const hexToRgb = (h) => { const m = /^#?([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec((h || "").trim()); return m ? [parseInt(m[1], 16), parseInt(m[2], 16), parseInt(m[3], 16)] : [128, 128, 160]; };
  const rgbToHex = (c) => "#" + c.map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0")).join("");
  const mix = (a, b, t) => { const x = hexToRgb(a), y = hexToRgb(b); return rgbToHex(x.map((v, i) => v + (y[i] - v) * t)); };
  // Luminance relative WCAG (0 = noir, 1 = blanc).
  const luminance = (hex) => { const [r, g, b] = hexToRgb(hex).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
  // Texte lisible sur la couleur d'accent.
  const readableOn = (hex) => luminance(hex) > 0.42 ? "#0b0d1a" : "#ffffff";
  const isDarkBg = (bg) => luminance(bg) < 0.35;

  // Palette complète à partir de deux couleurs : fond et texte. Clair ou sombre selon la luminance du fond.
  function derive(bg, text) {
    bg = isHex(bg) ? bg.toLowerCase() : PALETTES.graphite.bg;
    text = isHex(text) ? text.toLowerCase() : (isDarkBg(bg) ? "#e6e9f2" : "#151826");
    if (isDarkBg(bg)) {
      return { bg, elev: mix(bg, text, 0.05), sunken: mix(bg, "#000000", 0.3), hover: mix(bg, text, 0.09), border: mix(bg, text, 0.14), borderStrong: mix(bg, text, 0.26), text, dim: mix(bg, text, 0.72), faint: mix(bg, text, 0.45), ...DARK_STATUS };
    }
    return { bg, elev: mix(bg, "#ffffff", 0.7), sunken: mix(bg, "#000000", 0.045), hover: mix(bg, "#000000", 0.03), border: mix(bg, "#000000", 0.09), borderStrong: mix(bg, "#000000", 0.18), text, dim: mix(text, bg, 0.3), faint: mix(text, bg, 0.55), ...LIGHT_STATUS };
  }

  // Ne garde que les 14 jetons connus, en hexadécimal #rrggbb. Les valeurs absentes ou invalides sont
  // complétées par une palette dérivée (ou Graphite) : un thème importé ne peut jamais injecter autre chose qu'une couleur.
  function cleanCustom(obj) {
    const src = obj && typeof obj === "object" && !Array.isArray(obj) ? obj : {};
    const base = derive(isHex(src.bg) ? src.bg : undefined, isHex(src.text) ? src.text : undefined);
    const out = {};
    for (const [k] of TOKENS) out[k] = isHex(src[k]) ? src[k].toLowerCase() : base[k];
    return out;
  }

  // CSS libre : jamais de ressource externe (url non data:, @import), car l'interface doit rester hors ligne
  // et un thème importé ne doit pas pouvoir faire sortir de requête. Les échappements CSS sont décodés avant
  // le contrôle pour qu'on ne puisse pas écrire « u\72l( ».
  const FORBIDDEN = /@import|url\s*\(|image-set\s*\(|\bsrc\s*\(|expression\s*\(|javascript:|behavior\s*:|-moz-binding/i;
  const decodeCss = (s) => s.replace(/\\([0-9a-f]{1,6})\s?/gi, (_, h) => { const n = parseInt(h, 16); return n > 0 && n < 0x110000 ? String.fromCodePoint(n) : ""; }).replace(/\\([^\n])/g, "$1");
  function sanitizeCss(css) {
    let s = String(css == null ? "" : css).slice(0, MAX_CSS);
    const dec = decodeCss(s);
    if (FORBIDDEN.test(dec)) s = dec;
    s = s.replace(/<\/?style[^>]*>/gi, "").replace(/<!--|-->/g, "")
      .replace(/@import[^;{}]*;?/gi, "")
      .replace(/(-webkit-)?image-set\s*\([^)]*\)/gi, "none")
      .replace(/url\(\s*(?!["']?data:image\/)[^)]*\)/gi, 'url("data:,")')
      .replace(/expression\s*\([^)]*\)/gi, "none")
      .replace(/javascript:/gi, "").replace(/behavior\s*:[^;}]*;?/gi, "").replace(/-moz-binding\s*:[^;}]*;?/gi, "");
    return FORBIDDEN.test(decodeCss(s).replace(/url\(\s*["']?(data:image\/|data:,)[^)]*\)/gi, "")) ? "" : s;
  }

  // ---------- État ----------
  function sanitizeState(a) {
    const out = { ...DEFAULTS, ...a };
    if (!THEMES.some((t) => t[0] === out.theme)) out.theme = DEFAULTS.theme;
    if (!isHex(out.accent)) out.accent = DEFAULTS.accent;
    if (!["high", "balanced", "eco"].includes(out.bgQuality)) out.bgQuality = DEFAULTS.bgQuality;
    out.custom = out.custom ? cleanCustom(out.custom) : null;
    out.customCss = sanitizeCss(out.customCss);
    return out;
  }
  let current = { ...DEFAULTS };
  try { current = sanitizeState(JSON.parse(localStorage.getItem(KEY) || "{}")); } catch {}

  function resolvedTheme(a) {
    if (a.theme !== "auto") return a.theme;
    return window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "graphite";
  }
  // Couleurs réellement affichées pour un thème (point de départ du thème personnalisé).
  function snapshot(themeId) {
    const id = themeId === "auto" ? resolvedTheme({ theme: "auto" }) : themeId;
    if (id === "custom") return { ...(current.custom || PALETTES.graphite) };
    return { ...(PALETTES[id] || PALETTES.graphite) };
  }

  function applyCustomTokens(a) {
    const style = root.style;
    if (resolvedTheme(a) === "custom") {
      const c = a.custom || PALETTES.graphite;
      for (const [k, v] of TOKENS) style.setProperty(v, c[k]);
      const dark = isDarkBg(c.bg);
      style.setProperty("color-scheme", dark ? "dark" : "light");
      style.setProperty("--shadow", dark ? SHADOW_DARK : SHADOW_LIGHT);
    } else {
      for (const [, v] of TOKENS) style.removeProperty(v);
      style.removeProperty("color-scheme");
      style.removeProperty("--shadow");
    }
  }
  function applyCustomCss(css) {
    let el = document.getElementById("mcm-custom-css");
    if (!css) { if (el) el.remove(); return; }
    if (!el) { el = document.createElement("style"); el.id = "mcm-custom-css"; document.head.appendChild(el); }
    el.textContent = css;
  }

  function apply(a = current) {
    const theme = resolvedTheme(a);
    root.dataset.theme = theme === "graphite" ? "" : theme;
    root.dataset.font = a.font === "inter" ? "" : a.font;
    applyCustomTokens(a);
    root.style.setProperty("--accent", a.accent);
    root.style.setProperty("--accent-fg", readableOn(a.accent));
    root.style.setProperty("--radius", `${a.radius}px`);
    root.style.setProperty("--density", String(a.density));
    root.style.setProperty("--font-scale", String(a.fontScale));
    root.style.setProperty("--sidebar-w", `${a.sidebar}px`);
    root.style.setProperty("--bg-fx", String(a.bg === "none" ? 0 : 1));
    applyCustomCss(a.customCss);
    if (window.BackgroundFX) window.BackgroundFX.configure(a);
  }

  const persist = () => { try { localStorage.setItem(KEY, JSON.stringify(current)); } catch {} };

  window.Appearance = {
    ACCENTS, DEFAULTS, TOKENS, hexToRgb, isHex, derive, snapshot, cleanCustom, sanitizeCss, luminance,
    // Le thème « Personnalisé » affiche ses propres couleurs dans sa vignette.
    get THEMES() { return THEMES.map((t) => t[3] ? t : [t[0], t[1], t[2], (() => { const c = current.custom || snapshot(resolvedTheme(current)); return [c.bg, c.elev, c.hover]; })()]); },
    get: () => ({ ...current, custom: current.custom ? { ...current.custom } : null }),
    set(patch) {
      const next = { ...current, ...patch };
      // Premier passage en « Personnalisé » : on part des couleurs actuellement affichées.
      if (next.theme === "custom" && !next.custom) next.custom = snapshot(resolvedTheme(current));
      current = sanitizeState(next);
      persist(); apply();
    },
    // Modifie un seul jeton du thème personnalisé (active le thème au passage).
    setColor(token, hex) {
      if (!TOKENS.some((t) => t[0] === token) || !isHex(hex)) return;
      const base = current.custom || snapshot(resolvedTheme(current));
      this.set({ theme: "custom", custom: { ...base, [token]: hex } });
    },
    // Palette complète depuis deux couleurs.
    deriveFrom(bg, text) { this.set({ theme: "custom", custom: derive(bg, text) }); },
    reset() { current = { ...DEFAULTS }; try { localStorage.removeItem(KEY); } catch {} apply(); },
    exportJson() {
      const c = current.custom || snapshot(resolvedTheme(current));
      return JSON.stringify({ mcmanagerTheme: 1, accent: current.accent, colors: c, css: current.customCss }, null, 2);
    },
    // Importe un thème : tout est validé, un contenu hostile est neutralisé (couleurs hex uniquement, CSS filtré).
    importJson(text) {
      if (typeof text !== "string" || text.length > 100000) return { ok: false, error: "fichier trop volumineux ou invalide" };
      let obj;
      try { obj = JSON.parse(text); } catch { return { ok: false, error: "JSON invalide" }; }
      if (!obj || typeof obj !== "object" || obj.mcmanagerTheme !== 1) return { ok: false, error: "ce n'est pas un thème MCManager" };
      const patch = { theme: "custom", custom: cleanCustom(obj.colors), customCss: sanitizeCss(obj.css) };
      if (isHex(obj.accent)) patch.accent = obj.accent.toLowerCase();
      this.set(patch);
      return { ok: true };
    },
  };
  if (window.matchMedia) window.matchMedia("(prefers-color-scheme: light)").addEventListener?.("change", () => current.theme === "auto" && apply());
  apply();

  // ---------- Fond animé : matrice de points qui réagit à la souris ----------
  // Modes : « dots » (grille), « world » (planisphère en points), « none ».
  //
  // Pourquoi c'est fluide : le calcul coûte peu, c'est la composition de la page qui coûtait cher. On évite donc
  //  - de redessiner tout le canvas : seul un « rectangle sale » autour du curseur et des ondes est effacé puis redessiné ;
  //  - de parcourir tous les points : un index spatial (cases de 48 px) ne donne que ceux de la zone ;
  //  - de construire des chaînes CSS par point : les points voisins du curseur sont regroupés en 6 niveaux de lueur,
  //    chacun tracé en un seul chemin.
  // La boucle ne tourne que pendant une interaction : au repos, l'image est fixe et ne coûte rien.
  const TAU = Math.PI * 2, CELL = 48, LEVELS = 6, RIPPLE_MS = 900, SLOW_MS = 9, SLOW_FRAMES = 40;
  const QUALITY = { high: { dpr: 1.5, reach: 1 }, balanced: { dpr: 1.25, reach: 0.92 }, eco: { dpr: 1, reach: 0.75 } };
  const perf = window.__mcmPerf = { frames: 0, totalMs: 0, maxMs: 0, slowRun: 0, degraded: false, lastMs: 0 };

  const BG = {
    canvas: null, ctx: null, w: 0, h: 0, dpr: 1, mode: "dots", intensity: 0.55, interactive: true, quality: "balanced",
    px: new Float32Array(0), py: new Float32Array(0), cells: [], cols: 0, rows: 0, size: 1.1,
    mx: -9999, my: -9999, tx: -9999, ty: -9999, strength: 0, target: 0, raf: 0, ripples: [], prev: [],
    world: null, rgb: [128, 128, 160], accent: [109, 124, 255], layoutKey: "", layoutToken: 0, resizeTimer: 0,
    baseColor: "", levelColors: [], levelPaths: Array.from({ length: LEVELS }, () => []),

    get stats() { return { ...perf, quality: this.effectiveQuality(), points: this.px.length, avgMs: perf.frames ? perf.totalMs / perf.frames : 0 }; },
    effectiveQuality() { return perf.degraded ? "eco" : this.quality; },

    configure(a) {
      const reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      const q = QUALITY[a.bgQuality] ? a.bgQuality : "balanced";
      if (q !== this.quality) perf.degraded = false; // un changement explicite de qualité remet la dégradation à zéro
      this.mode = a.bg; this.intensity = a.bgIntensity / 100; this.quality = q;
      this.interactive = a.bgInteractive && a.motion !== false && !reduce;
      if (!this.canvas) return;
      this.canvas.style.display = a.bg === "none" ? "none" : "";
      const cs = getComputedStyle(root);
      this.rgb = hexToRgb(cs.getPropertyValue("--text-faint"));
      this.accent = hexToRgb(cs.getPropertyValue("--accent"));
      this.buildColors();
      this.layout();
    },

    init() {
      this.canvas = document.getElementById("bg-canvas");
      if (!this.canvas) return;
      this.ctx = this.canvas.getContext("2d", { alpha: true, desynchronized: true });
      window.addEventListener("resize", () => { clearTimeout(this.resizeTimer); this.resizeTimer = setTimeout(() => this.layout(), 150); });
      window.addEventListener("pointermove", (e) => { this.tx = e.clientX; this.ty = e.clientY; if (this.mx < -999) { this.mx = this.tx; this.my = this.ty; } this.target = 1; this.kick(); }, { passive: true });
      window.addEventListener("pointerdown", (e) => { if (this.interactive) { this.ripples.push({ x: e.clientX, y: e.clientY, t0: performance.now() }); this.kick(); } }, { passive: true });
      document.addEventListener("pointerleave", () => { this.target = 0; this.kick(); });
      // Onglet masqué : on arrête tout ; au retour, une image propre.
      document.addEventListener("visibilitychange", () => {
        if (document.hidden) { cancelAnimationFrame(this.raf); this.raf = 0; this.ripples = []; this.target = 0; this.strength = 0; }
        else this.paintAll();
      });
      this.configure(current);
    },

    // Couleurs précalculées : base + un dégradé base → accent par niveau de lueur.
    buildColors() {
      const I = this.intensity, [br, bg, bb] = this.rgb, [ar, ag, ab] = this.accent;
      const baseA = 0.13 + 0.34 * I;
      this.baseA = baseA;
      this.baseColor = `rgba(${br},${bg},${bb},${baseA})`;
      this.levelColors = [];
      for (let l = 0; l < LEVELS; l++) {
        const e = (l + 0.5) / LEVELS;
        this.levelColors.push(`rgba(${Math.round(br + (ar - br) * e)},${Math.round(bg + (ag - bg) * e)},${Math.round(bb + (ab - bb) * e)},${Math.min(0.95, baseA + e * (0.55 + 0.3 * I)).toFixed(3)})`);
      }
    },
    reach() { return (150 + 90 * this.intensity) * QUALITY[this.effectiveQuality()].reach; },

    async layout() {
      if (!this.canvas || this.mode === "none") return;
      const dpr = Math.min(window.devicePixelRatio || 1, QUALITY[this.effectiveQuality()].dpr);
      const w = window.innerWidth, h = window.innerHeight;
      const key = `${this.mode}|${w}|${h}|${dpr}`;
      if (key === this.layoutKey) { this.paintAll(); return; } // seules les couleurs ont changé
      const token = ++this.layoutToken;
      let list = [], size = 1.1;
      if (this.mode === "world") {
        if (!this.world) { try { this.world = await (await fetch("assets/world-dots.json")).json(); } catch { this.world = null; } }
        if (token !== this.layoutToken) return; // une configuration plus récente a pris le relais
        if (this.world) {
          const { cols, rows, data } = this.world;
          const cell = Math.max(w / cols, (h * 0.9) / rows);
          const ox = (w - cell * cols) / 2, oy = (h - cell * rows) / 2;
          for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) if (data[r][c] === "1") list.push(ox + (c + 0.5) * cell, oy + (r + 0.5) * cell);
          size = Math.max(1.1, cell * 0.2);
        }
      } else {
        const gap = 28;
        for (let y = gap / 2; y < h; y += gap) for (let x = gap / 2; x < w; x += gap) list.push(x, y);
      }
      this.w = w; this.h = h; this.dpr = dpr; this.size = size; this.layoutKey = key;
      this.canvas.width = Math.round(w * dpr); this.canvas.height = Math.round(h * dpr);
      this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const n = list.length / 2;
      this.px = new Float32Array(n); this.py = new Float32Array(n);
      this.cols = Math.ceil(w / CELL); this.rows = Math.ceil(h / CELL);
      this.cells = Array.from({ length: this.cols * this.rows }, () => []);
      for (let i = 0; i < n; i++) {
        this.px[i] = list[2 * i]; this.py[i] = list[2 * i + 1];
        const cx = Math.min(this.cols - 1, Math.max(0, Math.floor(this.px[i] / CELL))), cy = Math.min(this.rows - 1, Math.max(0, Math.floor(this.py[i] / CELL)));
        this.cells[cy * this.cols + cx].push(i);
      }
      this.prev = [];
      this.paintAll();
    },

    paintAll() { if (this.ctx && this.mode !== "none") this.paint([{ x0: 0, y0: 0, x1: this.w, y1: this.h }], performance.now()); },

    kick() { if (!this.raf && this.interactive && !document.hidden) this.raf = requestAnimationFrame((t) => this.frame(t)); },

    frame(now) {
      this.raf = 0;
      const t0 = performance.now();
      this.strength += (this.target - this.strength) * 0.22;
      this.mx += (this.tx - this.mx) * 0.55; this.my += (this.ty - this.my) * 0.55; // suivi quasi direct : pas de latence ressentie
      this.ripples = this.ripples.filter((r) => now - r.t0 < RIPPLE_MS);
      const cur = this.dirtyRects(now);
      this.paint(this.merge(cur.concat(this.prev)), now);
      this.prev = cur;
      this.measure(performance.now() - t0);
      const settling = Math.abs(this.target - this.strength) > 0.01 || Math.hypot(this.tx - this.mx, this.ty - this.my) > 0.6 || this.ripples.length;
      if (settling) this.raf = requestAnimationFrame((t) => this.frame(t));
    },

    // Mesure + dégradation automatique : si le dessin dépasse 9 ms pendant 40 images de suite, on passe en « eco ».
    measure(ms) {
      perf.frames++; perf.totalMs += ms; perf.lastMs = ms; if (ms > perf.maxMs) perf.maxMs = ms;
      perf.slowRun = ms > SLOW_MS ? perf.slowRun + 1 : 0;
      if (perf.slowRun >= SLOW_FRAMES && this.quality !== "eco" && !perf.degraded) {
        perf.degraded = true; this.layoutKey = ""; this.layout();
      }
    },

    // Zones à redessiner : autour du curseur (tant qu'il agit) et autour de chaque onde de choc.
    dirtyRects(now) {
      const out = [], pad = 16;
      if (this.strength > 0.01 || this.target > 0) {
        const r = this.reach() + pad + this.size * 3;
        out.push({ x0: this.mx - r, y0: this.my - r, x1: this.mx + r, y1: this.my + r });
      }
      for (const rp of this.ripples) {
        const r = ((now - rp.t0) / RIPPLE_MS) * 340 + 46 + pad;
        out.push({ x0: rp.x - r, y0: rp.y - r, x1: rp.x + r, y1: rp.y + r });
      }
      return out;
    },
    // Fusionne les rectangles qui se touchent : deux zones disjointes ne partagent aucun point (pas de double tracé).
    merge(rects) {
      const list = rects.map((r) => ({ x0: Math.max(0, r.x0), y0: Math.max(0, r.y0), x1: Math.min(this.w, r.x1), y1: Math.min(this.h, r.y1) })).filter((r) => r.x1 > r.x0 && r.y1 > r.y0);
      for (let again = true; again;) {
        again = false;
        outer: for (let i = 0; i < list.length; i++) for (let j = i + 1; j < list.length; j++) {
          const a = list[i], b = list[j];
          if (a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1) {
            list[i] = { x0: Math.min(a.x0, b.x0), y0: Math.min(a.y0, b.y0), x1: Math.max(a.x1, b.x1), y1: Math.max(a.y1, b.y1) };
            list.splice(j, 1); again = true; break outer;
          }
        }
      }
      return list;
    },

    paint(rects, now) {
      const ctx = this.ctx; if (!ctx) return;
      for (const r of rects) this.paintRect(ctx, r, now);
    },

    paintRect(ctx, r, now) {
      const { px, py, cells, cols, rows, size, ripples } = this;
      const reach = this.reach(), R2 = reach * reach, s = this.strength, I = this.intensity, mx = this.mx, my = this.my, half = size / 2;
      ctx.save();
      ctx.beginPath(); ctx.rect(r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0); ctx.clip(); // le tracé reste strictement dans la zone effacée
      ctx.clearRect(r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0);
      const c0 = Math.max(0, Math.floor((r.x0 - 16) / CELL)), c1 = Math.min(cols - 1, Math.floor((r.x1 + 16) / CELL));
      const r0 = Math.max(0, Math.floor((r.y0 - 16) / CELL)), r1 = Math.min(rows - 1, Math.floor((r.y1 + 16) / CELL));
      const levels = this.levelPaths; for (const l of levels) l.length = 0;
      ctx.beginPath();
      for (let cy = r0; cy <= r1; cy++) for (let cx = c0; cx <= c1; cx++) {
        const cell = cells[cy * cols + cx];
        for (let n = 0; n < cell.length; n++) {
          const i = cell[n]; let x = px[i], y = py[i];
          for (let k = 0; k < ripples.length; k++) { // onde de choc au clic
            const rp = ripples[k], age = (now - rp.t0) / RIPPLE_MS, d = Math.hypot(x - rp.x, y - rp.y), dd = d - age * 340;
            if (d && Math.abs(dd) < 46) { const push = Math.cos((dd / 46) * Math.PI / 2) * (1 - age) * 11 / d; x += (x - rp.x) * push; y += (y - rp.y) * push; }
          }
          const dx = x - mx, dy = y - my, d2 = dx * dx + dy * dy;
          if (s > 0.01 && d2 < R2) {
            const dist = Math.sqrt(d2), e = (1 - dist / reach) ** 2 * s, d = dist || 1;
            const lvl = levels[Math.min(LEVELS - 1, Math.floor(e * LEVELS))];
            lvl.push(x + (dx / d) * e * 12, y + (dy / d) * e * 12, e);
          } else ctx.rect(x - half, y - half, size, size);
        }
      }
      ctx.fillStyle = this.baseColor; ctx.fill();
      for (let l = 0; l < LEVELS; l++) {
        const pts = levels[l]; if (!pts.length) continue;
        const rad = size * (0.6 + ((l + 0.5) / LEVELS) * 1.6);
        ctx.beginPath();
        for (let n = 0; n < pts.length; n += 3) { ctx.moveTo(pts[n] + rad, pts[n + 1]); ctx.arc(pts[n], pts[n + 1], rad, 0, TAU); }
        ctx.fillStyle = this.levelColors[l]; ctx.fill();
      }
      ctx.restore();
    },
  };
  window.BackgroundFX = BG;
  const start = () => { BG.init(); };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start); else start();
})();
