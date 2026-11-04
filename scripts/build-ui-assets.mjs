import fs from "fs"; import path from "path";
import { feature } from "topojson-client"; import { geoContains } from "d3-geo";
const OUT = new URL("../web/assets", import.meta.url).pathname; // npm i lucide-static @fontsource-variable/inter @fontsource-variable/jetbrains-mono world-atlas topojson-client d3-geo
fs.mkdirSync(OUT + "/fonts", { recursive: true });

// 1. Sprite d'icônes (Lucide, licence ISC) : seulement celles dont l'interface a besoin.
const names = `layout-dashboard server terminal folder folder-open puzzle store box shield-check sliders-horizontal archive chart-line waypoints network globe users user settings book-open bot blocks play square rotate-cw refresh-cw upload download save trash-2 plus x check triangle-alert send link key-round eye search clipboard-copy scroll-text bell sparkles flask-conical stethoscope eraser arrow-up arrow-down file-text file-pen package zap hard-drive cpu palette log-out chevron-down chevron-right circle-check circle-alert info copy external-link pencil power wifi wifi-off map-pin moon sun monitor panel-left menu activity lock shield messages-square history database cloud sliders-vertical rotate-ccw image type layers move-horizontal gauge radio hash dot ellipsis-vertical file-archive file-up file-down mouse-pointer-2 sparkle wand-sparkles`.split(/\s+/);
let sprite = `<svg xmlns="http://www.w3.org/2000/svg" style="display:none">\n`;
const missing = [];
for (const n of names) {
  const f = `node_modules/lucide-static/icons/${n}.svg`;
  if (!fs.existsSync(f)) { missing.push(n); continue; }
  const inner = fs.readFileSync(f, "utf8").replace(/<!--[\s\S]*?-->/g, "").replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "").trim();
  sprite += `<symbol id="${n}" viewBox="0 0 24 24">${inner}</symbol>\n`;
}
sprite += `</svg>\n`;
fs.writeFileSync(OUT + "/icons.svg", sprite);
console.log("icons:", names.length - missing.length, "missing:", missing.join(",") || "none", "bytes:", sprite.length);

// 2. Polices auto-hébergées (OFL) : fonctionne hors ligne / sur un réseau local sans Internet.
const fontsDir = "node_modules/@fontsource-variable";
for (const [pkg, file, out] of [["inter", "inter-latin-wght-normal.woff2", "inter.woff2"], ["jetbrains-mono", "jetbrains-mono-latin-wght-normal.woff2", "jetbrains-mono.woff2"]]) {
  fs.copyFileSync(`${fontsDir}/${pkg}/files/${file}`, `${OUT}/fonts/${out}`);
}

// 3. Carte du monde en points : grille équirectangulaire, un point si la case est sur une terre émergée.
const topo = JSON.parse(fs.readFileSync("node_modules/world-atlas/land-110m.json", "utf8"));
const land = feature(topo, topo.objects.land);
const SETS = [["world-dots.json",160,64],["world-dots-hi.json",480,192]];
for (const [file, COLS, ROWS] of SETS) {
const COLS_ = COLS; // 2.25° par case en longitude, ~2.3° en latitude (de 84°N à 62°S : sans l'Antarctique)
const LAT_TOP = 84, LAT_BOTTOM = -62;

const rows = []; let dots = 0;
for (let r = 0; r < ROWS; r++) {
  const lat = LAT_TOP - ((r + 0.5) / ROWS) * (LAT_TOP - LAT_BOTTOM);
  let line = "";
  for (let c = 0; c < COLS; c++) {
    const lon = -180 + ((c + 0.5) / COLS) * 360;
    const on = geoContains(land, [lon, lat]);
    line += on ? "1" : "0"; if (on) dots++;
  }
  rows.push(line);
}
fs.writeFileSync(OUT + "/" + file, JSON.stringify({ cols: COLS, rows: ROWS, latTop: LAT_TOP, latBottom: LAT_BOTTOM, data: rows }));
console.log(file, "dots:", dots);
}
