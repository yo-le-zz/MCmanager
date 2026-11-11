// Tests des fonctions pures de web/ui.js (thème personnalisé, import hostile, CSS libre).
// Usage : node scripts/test-ui.js
"use strict";
const fs = require("fs"), vm = require("vm"), assert = require("assert"), path = require("path");

function load() {
  const store = {};
  const styleProps = {};
  const head = { children: [], appendChild(el) { this.children.push(el); el.parentNode = this; } };
  const documentStub = {
    readyState: "complete", hidden: false, head,
    documentElement: { dataset: {}, style: { setProperty: (k, v) => { styleProps[k] = v; }, removeProperty: (k) => { delete styleProps[k]; } } },
    getElementById: (id) => (id === "mcm-custom-css" ? head.children.find((c) => c.id === id) || null : null),
    createElement: () => ({ id: "", textContent: "", remove() { head.children = head.children.filter((c) => c !== this); } }),
    addEventListener() {},
  };
  const win = {
    matchMedia: () => ({ matches: false, addEventListener() {} }), addEventListener() {},
    localStorage: { getItem: (k) => store[k] || null, setItem: (k, v) => { store[k] = v; }, removeItem: (k) => { delete store[k]; } },
  };
  const ctx = { window: win, document: documentStub, localStorage: win.localStorage, getComputedStyle: () => ({ getPropertyValue: () => "#888888" }), performance: { now: () => 0 }, requestAnimationFrame: () => 1, cancelAnimationFrame() {}, setTimeout, clearTimeout, console };
  win.window = win; Object.assign(win, ctx);
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(__dirname, "../web/ui.js"), "utf8"), ctx);
  return { A: win.Appearance, styleProps, head, store };
}

let n = 0;
const test = (name, fn) => { fn(); n++; console.log("ok -", name); };
const HEX = /^#[0-9a-f]{6}$/;

test("derive : palette complète, sombre ou claire selon la luminance du fond", () => {
  const { A } = load();
  const dark = A.derive("#101820", "#eeeeee"), light = A.derive("#f5f0e6", "#1a1a1a");
  for (const [k] of A.TOKENS) { assert.match(dark[k], HEX, k); assert.match(light[k], HEX, k); }
  assert.ok(A.luminance(dark.elev) > A.luminance(dark.bg), "carte plus claire que le fond en sombre");
  assert.ok(A.luminance(light.elev) >= A.luminance(light.bg), "carte au moins aussi claire en clair");
  assert.strictEqual(light.ok, "#059669"); assert.strictEqual(dark.ok, "#34d399");
});

test("cleanCustom : hex #rrggbb uniquement, le reste est remplacé", () => {
  const { A } = load();
  const c = A.cleanCustom({ bg: "#112233", text: "red", elev: "url(http://x)", border: "#ABCDEF", evil: "x", __proto__: { bg: "#000000" } });
  assert.strictEqual(c.bg, "#112233"); assert.strictEqual(c.border, "#abcdef");
  for (const [k] of A.TOKENS) assert.match(c[k], HEX, k);
  assert.ok(!("evil" in c));
  assert.strictEqual(Object.keys(A.cleanCustom(null)).length, 14);
});

test("sanitizeCss : pas de ressource externe, même échappée", () => {
  const { A } = load();
  const bad = ["@import 'https://evil.test/x.css';", "a{background:url(https://evil.test/p.png)}", "a{background:u\\72l(//evil.test/p)}", "a{background:image-set('//evil.test/p' 1x)}", "</style><script>alert(1)</script>", "a{width:expression(alert(1))}"];
  for (const css of bad) {
    const out = A.sanitizeCss(css);
    assert.ok(!/evil\.test|@import|<\/style|expression\(/i.test(out), `resté dans : ${out}`);
  }
  assert.strictEqual(A.sanitizeCss(".card{border-radius:4px}"), ".card{border-radius:4px}");
  assert.ok(A.sanitizeCss('a{background:url("data:image/png;base64,AAAA")}').includes("data:image/png"));
  assert.ok(A.sanitizeCss("x".repeat(50000)).length <= 20000);
});

test("importJson : thème valide appliqué, accent et CSS validés", () => {
  const { A, styleProps, head } = load();
  const exported = JSON.stringify({ mcmanagerTheme: 1, accent: "#ff0000", colors: { bg: "#202020", text: "#fafafa" }, css: ".card{opacity:.9}" });
  assert.strictEqual(A.importJson(exported).ok, true); // pas de deepStrictEqual : objets issus d'un autre contexte vm
  assert.strictEqual(A.get().theme, "custom"); assert.strictEqual(A.get().accent, "#ff0000");
  assert.strictEqual(styleProps["--bg"], "#202020"); assert.strictEqual(styleProps["color-scheme"], "dark");
  assert.strictEqual(head.children[0].textContent, ".card{opacity:.9}");
});

test("importJson : contenu hostile neutralisé ou refusé", () => {
  const { A, styleProps, head } = load();
  assert.strictEqual(A.importJson("pas du json").ok, false);
  assert.strictEqual(A.importJson(JSON.stringify({ colors: { bg: "#000000" } })).ok, false); // marqueur absent
  assert.strictEqual(A.importJson("x".repeat(200000)).ok, false);
  const r = A.importJson(JSON.stringify({ mcmanagerTheme: 1, accent: "javascript:alert(1)", colors: { bg: "url(//evil.test)", text: "</style>", ok: "expression(1)" }, css: "@import url(//evil.test/x.css); a{background:url(//evil.test/p)}" }));
  assert.strictEqual(r.ok, true);
  assert.strictEqual(A.get().accent, "#6d7cff", "accent invalide ignoré");
  for (const [, v] of A.TOKENS) assert.match(styleProps[v], HEX, v);
  assert.ok(!head.children.some((c) => /evil\.test/.test(c.textContent)));
});

test("snapshot, thème personnalisé et retour à un thème intégré", () => {
  const { A, styleProps } = load();
  assert.strictEqual(A.snapshot("light").bg, "#f6f7fa");
  A.set({ theme: "light" });
  A.setColor("bg", "#ffeedd");                         // part des couleurs affichées (Clair)
  assert.strictEqual(A.get().theme, "custom");
  assert.strictEqual(A.get().custom.elev, "#ffffff");
  assert.strictEqual(styleProps["color-scheme"], "light");
  A.set({ theme: "midnight" });
  assert.ok(!("--bg" in styleProps) && !("color-scheme" in styleProps), "jetons en ligne retirés");
  assert.ok(A.get().custom, "le thème personnalisé reste mémorisé");
});

test("export puis import : aller-retour identique", () => {
  const a = load().A; a.deriveFrom("#1b1030", "#f0e8ff"); a.set({ accent: "#abcdef" });
  const json = a.exportJson();
  const b = load().A; assert.strictEqual(b.importJson(json).ok, true);
  assert.strictEqual(JSON.stringify(b.get().custom), JSON.stringify(a.get().custom)); assert.strictEqual(b.get().accent, "#abcdef");
});

test("style.css déclare les 14 jetons (miroir des palettes)", () => {
  const css = fs.readFileSync(path.join(__dirname, "../web/style.css"), "utf8");
  const { A } = load();
  for (const [, v] of A.TOKENS) assert.ok(css.includes(v + ":"), v);
  const g = css.match(/:root \{[\s\S]*?\n\}/)[0], P = A.snapshot("graphite");
  for (const [k, v] of A.TOKENS) assert.ok(new RegExp(v.replace(/[-]/g, "\\-") + ":\\s*" + P[k] + "\\b", "i").test(g), `${v} ≠ palette Graphite`);
});

console.log(`\n${n} tests UI OK`);
