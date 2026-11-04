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

  // ---------- Apparence ----------
  const THEMES = [
    ["auto", "Automatique", "Suit le thème du système", ["#0e1015", "#14171e", "#f6f7fa"]],
    ["graphite", "Graphite", "Sombre neutre", ["#0e1015", "#14171e", "#242936"]],
    ["midnight", "Minuit", "Sombre bleuté", ["#070b16", "#0d1424", "#1a2744"]],
    ["forest", "Forêt", "Sombre vert", ["#0b110e", "#111a15", "#1f3026"]],
    ["light", "Clair", "Lumineux", ["#f6f7fa", "#ffffff", "#e2e5ec"]],
    ["contrast", "Contraste élevé", "Accessibilité", ["#000000", "#0a0a0a", "#5a5a5a"]],
  ];
  const ACCENTS = [["Indigo", "#6d7cff"], ["Bleu", "#3b82f6"], ["Cyan", "#06b6d4"], ["Émeraude", "#10b981"], ["Citron", "#a3e635"], ["Ambre", "#f59e0b"], ["Orange", "#f97316"], ["Rose", "#f43f5e"], ["Violet", "#a78bfa"], ["Ardoise", "#94a3b8"]];
  const DEFAULTS = { theme: "graphite", accent: "#6d7cff", radius: 10, density: 1, font: "inter", fontScale: 1, sidebar: 248, bg: "dots", bgIntensity: 55, bgInteractive: true, motion: true };

  let current = { ...DEFAULTS };
  try { current = { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY) || "{}") }; } catch {}

  const hexToRgb = (h) => { const m = /^#?([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec((h || "").trim()); return m ? [parseInt(m[1], 16), parseInt(m[2], 16), parseInt(m[3], 16)] : [128, 128, 160]; };
  // Texte lisible sur la couleur d'accent (luminance relative WCAG).
  const readableOn = (hex) => { const [r, g, b] = hexToRgb(hex).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.42 ? "#0b0d1a" : "#ffffff"; };

  function resolvedTheme(a) {
    if (a.theme !== "auto") return a.theme;
    return window.matchMedia && window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "graphite";
  }

  function apply(a = current) {
    root.dataset.theme = resolvedTheme(a) === "graphite" ? "" : resolvedTheme(a);
    root.dataset.font = a.font === "inter" ? "" : a.font;
    root.style.setProperty("--accent", a.accent);
    root.style.setProperty("--accent-fg", readableOn(a.accent));
    root.style.setProperty("--radius", `${a.radius}px`);
    root.style.setProperty("--density", String(a.density));
    root.style.setProperty("--font-scale", String(a.fontScale));
    root.style.setProperty("--sidebar-w", `${a.sidebar}px`);
    root.style.setProperty("--bg-fx", String(a.bg === "none" ? 0 : 1));
    if (window.BackgroundFX) window.BackgroundFX.configure(a);
  }

  window.Appearance = {
    THEMES, ACCENTS, DEFAULTS, hexToRgb,
    get: () => ({ ...current }),
    set(patch) { current = { ...current, ...patch }; try { localStorage.setItem(KEY, JSON.stringify(current)); } catch {} apply(); },
    reset() { current = { ...DEFAULTS }; try { localStorage.removeItem(KEY); } catch {} apply(); },
  };
  if (window.matchMedia) window.matchMedia("(prefers-color-scheme: light)").addEventListener?.("change", () => current.theme === "auto" && apply());
  apply();

  // ---------- Fond animé : matrice de points qui réagit à la souris ----------
  // Modes : « dots » (grille), « world » (planisphère en points), « none ».
  // La boucle d'animation ne tourne que pendant une interaction : au repos, l'image est fixe et ne coûte rien.
  const BG = {
    canvas: null, ctx: null, w: 0, h: 0, dpr: 1, pts: [], mode: "dots", intensity: 0.55, interactive: true,
    mx: -9999, my: -9999, tx: -9999, ty: -9999, strength: 0, target: 0, raf: 0, last: 0, ripples: [], world: null, rgb: [128, 128, 160], accent: [109, 124, 255],

    configure(a) {
      this.mode = a.bg; this.intensity = a.bgIntensity / 100;
      const reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      this.interactive = a.bgInteractive && a.motion !== false && !reduce;
      if (!this.canvas) return;
      this.canvas.style.display = a.bg === "none" ? "none" : "";
      const cs = getComputedStyle(root);
      this.rgb = hexToRgb(cs.getPropertyValue("--text-faint"));
      this.accent = hexToRgb(cs.getPropertyValue("--accent"));
      this.layout();
    },

    init() {
      this.canvas = document.getElementById("bg-canvas");
      if (!this.canvas) return;
      this.ctx = this.canvas.getContext("2d");
      window.addEventListener("resize", () => this.layout());
      window.addEventListener("pointermove", (e) => { this.tx = e.clientX; this.ty = e.clientY; if (this.mx < -999) { this.mx = this.tx; this.my = this.ty; } this.target = 1; this.kick(); }, { passive: true });
      window.addEventListener("pointerdown", (e) => { if (this.interactive) { this.ripples.push({ x: e.clientX, y: e.clientY, t0: performance.now() }); this.kick(); } }, { passive: true });
      document.addEventListener("pointerleave", () => { this.target = 0; this.kick(); });
      this.configure(current);
    },

    async layout() {
      if (!this.canvas || this.mode === "none") return;
      this.dpr = Math.min(window.devicePixelRatio || 1, 2);
      this.w = window.innerWidth; this.h = window.innerHeight;
      this.canvas.width = this.w * this.dpr; this.canvas.height = this.h * this.dpr;
      this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
      this.pts = [];
      if (this.mode === "world") {
        if (!this.world) { try { this.world = await (await fetch("assets/world-dots.json")).json(); } catch { this.world = null; } }
        if (this.world) {
          const { cols, rows, data } = this.world;
          const cell = Math.max(this.w / cols, (this.h * 0.9) / rows);
          const ox = (this.w - cell * cols) / 2, oy = (this.h - cell * rows) / 2;
          for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) if (data[r][c] === "1") this.pts.push([ox + (c + 0.5) * cell, oy + (r + 0.5) * cell]);
          this.size = Math.max(1.1, cell * 0.2);
        }
      } else {
        const gap = 28; this.size = 1.1;
        for (let y = gap / 2; y < this.h; y += gap) for (let x = gap / 2; x < this.w; x += gap) this.pts.push([x, y]);
      }
      this.draw(performance.now());
    },

    kick() { if (!this.raf && this.interactive) this.raf = requestAnimationFrame((t) => this.frame(t)); },

    frame(now) {
      this.raf = 0;
      this.strength += (this.target - this.strength) * 0.12;
      this.mx += (this.tx - this.mx) * 0.2; this.my += (this.ty - this.my) * 0.2;
      this.ripples = this.ripples.filter((r) => now - r.t0 < 900);
      this.draw(now);
      const settling = Math.abs(this.target - this.strength) > 0.01 || Math.hypot(this.tx - this.mx, this.ty - this.my) > 0.6 || this.ripples.length;
      if (settling) this.raf = requestAnimationFrame((t) => this.frame(t));
    },

    draw(now) {
      const ctx = this.ctx; if (!ctx) return;
      ctx.clearRect(0, 0, this.w, this.h);
      const I = this.intensity, [br, bg, bb] = this.rgb, [ar, ag, ab] = this.accent;
      const baseA = 0.13 + 0.34 * I, R = 150 + 90 * I, R2 = R * R, s = this.strength, size = this.size || 1.1;
      ctx.fillStyle = `rgba(${br},${bg},${bb},${baseA})`;
      const near = [];
      for (const p of this.pts) {
        let [x, y] = p, push = 0;
        for (const rp of this.ripples) { // onde de choc au clic
          const age = (now - rp.t0) / 900, ring = age * 340, d = Math.hypot(x - rp.x, y - rp.y), dd = d - ring;
          if (Math.abs(dd) < 46) push += Math.cos((dd / 46) * Math.PI / 2) * (1 - age) * 11 * (d ? 1 / d : 0);
          if (push && d) { x += (x - rp.x) * push; y += (y - rp.y) * push; push = 0; }
        }
        const dx = x - this.mx, dy = y - this.my, d2 = dx * dx + dy * dy;
        if (s > 0.01 && d2 < R2) { near.push([x, y, 1 - Math.sqrt(d2) / R, dx, dy]); continue; }
        ctx.fillRect(x - size / 2, y - size / 2, size, size);
      }
      for (const [x, y, k, dx, dy] of near) { // points proches du curseur : repoussés, plus gros, teintés d'accent
        const e = k * k * s, d = Math.hypot(dx, dy) || 1;
        const px = x + (dx / d) * e * 12, py = y + (dy / d) * e * 12;
        ctx.fillStyle = `rgba(${Math.round(br + (ar - br) * e)},${Math.round(bg + (ag - bg) * e)},${Math.round(bb + (ab - bb) * e)},${Math.min(0.95, baseA + e * (0.55 + 0.3 * I))})`;
        ctx.beginPath(); ctx.arc(px, py, size * (0.6 + e * 1.6), 0, 6.2832); ctx.fill();
      }
    },
  };
  window.BackgroundFX = BG;
  const start = () => { BG.init(); };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start); else start();
})();
