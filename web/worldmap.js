/* Carte du monde en points (à la manière des pages d'accueil de GitLab/Vercel) :
 * terres émergées en points, machines en repères pulsants, flux de données animés entre elles.
 * Les points réagissent au curseur comme le fond de l'application. Aucune bibliothèque externe :
 * le planisphère vient de assets/world-dots.json (Natural Earth, domaine public).
 */
(function () {
  "use strict";
  // Deux résolutions : 2,25° (vue monde, léger) et 0,75° (vue zoomée sur une région, sinon les points deviennent des blocs).
  let worldData = null, worldHi = null;
  const loadWorld = async () => worldData || (worldData = await (await fetch("assets/world-dots.json")).json());
  const loadHi = async () => worldHi || (worldHi = await (await fetch("assets/world-dots-hi.json")).json());

  // Villes proposées pour placer une machine (nom, latitude, longitude)
  const CITIES = [["Paris", 48.86, 2.35], ["Londres", 51.51, -0.13], ["Francfort", 50.11, 8.68], ["Amsterdam", 52.37, 4.9], ["Madrid", 40.42, -3.7], ["Rome", 41.9, 12.5], ["Stockholm", 59.33, 18.07], ["Varsovie", 52.23, 21.01], ["Helsinki", 60.17, 24.94], ["Istanbul", 41.01, 28.98], ["Moscou", 55.76, 37.62], ["Reykjavik", 64.15, -21.94], ["Lisbonne", 38.72, -9.14], ["Zurich", 47.37, 8.54], ["Bruxelles", 50.85, 4.35], ["Montréal", 45.5, -73.57], ["New York", 40.71, -74.0], ["Washington", 38.9, -77.04], ["Chicago", 41.88, -87.63], ["Dallas", 32.78, -96.8], ["Denver", 39.74, -104.99], ["Los Angeles", 34.05, -118.24], ["San Francisco", 37.77, -122.42], ["Seattle", 47.61, -122.33], ["Vancouver", 49.28, -123.12], ["Mexico", 19.43, -99.13], ["Miami", 25.76, -80.19], ["Toronto", 43.65, -79.38], ["São Paulo", -23.55, -46.63], ["Buenos Aires", -34.6, -58.38], ["Santiago", -33.45, -70.67], ["Bogota", 4.71, -74.07], ["Lima", -12.05, -77.04], ["Le Caire", 30.04, 31.24], ["Lagos", 6.52, 3.38], ["Nairobi", -1.29, 36.82], ["Johannesburg", -26.2, 28.05], ["Casablanca", 33.57, -7.59], ["Dubaï", 25.2, 55.27], ["Tel Aviv", 32.09, 34.78], ["Mumbai", 19.08, 72.88], ["Delhi", 28.61, 77.21], ["Bangalore", 12.97, 77.59], ["Singapour", 1.35, 103.82], ["Jakarta", -6.21, 106.85], ["Bangkok", 13.76, 100.5], ["Hong Kong", 22.32, 114.17], ["Shanghai", 31.23, 121.47], ["Pékin", 39.9, 116.4], ["Séoul", 37.57, 126.98], ["Tokyo", 35.68, 139.69], ["Osaka", 34.69, 135.5], ["Manille", 14.6, 120.98], ["Sydney", -33.87, 151.21], ["Melbourne", -37.81, 144.96], ["Perth", -31.95, 115.86], ["Auckland", -36.85, 174.76], ["Saint-Denis (La Réunion)", -20.88, 55.45], ["Fort-de-France", 14.6, -61.07], ["Papeete", -17.54, -149.57]];

  function mount(container, opts) {
    const o = { markers: [], arcs: [], height: 0, ...opts };
    container.classList.add("worldmap");
    container.innerHTML = '<canvas></canvas><div class="worldmap-tip"></div><div class="worldmap-legend"></div><button class="worldmap-fit btn-ghost btn-sm" type="button"></button>';
    const fitBtn = container.querySelector(".worldmap-fit");
    const canvas = container.querySelector("canvas"), tip = container.querySelector(".worldmap-tip"), ctx = canvas.getContext("2d");
    const legend = container.querySelector(".worldmap-legend");
    legend.innerHTML = `<span><i class="dot on"></i> ${o.legendOnline || "En ligne"}</span><span><i class="dot"></i> ${o.legendOffline || "Hors ligne"}</span>`;
    let W = 0, H = 0, world = null, mx = -999, my = -999, raf = 0, fit = o.fit !== false;
    let hi = null;
    let view = null; // {lon0, lon1, lat0, lat1} : fenêtre affichée (monde entier ou zoom sur les machines)
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const css = (v) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
    const rgb = (h) => window.Appearance.hexToRgb(h);

    const project = (lat, lon) => [((lon - view.lon0) / (view.lon1 - view.lon0)) * W, ((view.lat0 - lat) / (view.lat0 - view.lat1)) * H];

    // Cadre le monde entier, ou zoome sur les machines placées (pas moins de 50° de large, avec marge).
    function computeView() {
      const full = { lon0: -180, lon1: 180, lat0: world.latTop, lat1: world.latBottom };
      const pts = positioned();
      if (!fit || !pts.length) return full;
      const lons = pts.map((m) => m.lon), lats = pts.map((m) => m.lat);
      let cx = (Math.min(...lons) + Math.max(...lons)) / 2, cy = (Math.min(...lats) + Math.max(...lats)) / 2;
      const ratio = H / W; // degrés de latitude par degré de longitude à l'écran
      let lonSpan = Math.max(70, (Math.max(...lons) - Math.min(...lons)) * 1.6, ((Math.max(...lats) - Math.min(...lats)) * 1.6) / ratio);
      if (lonSpan >= 300) return full;
      const latSpan = lonSpan * ratio;
      cx = Math.min(180 - lonSpan / 2, Math.max(-180 + lonSpan / 2, cx));
      cy = Math.min(world.latTop - latSpan / 2, Math.max(world.latBottom + latSpan / 2, cy));
      return { lon0: cx - lonSpan / 2, lon1: cx + lonSpan / 2, lat0: cy + latSpan / 2, lat1: cy - latSpan / 2 };
    }
    function size() {
      W = container.clientWidth || 800; H = W * (world ? world.rows / world.cols : 0.4);
      canvas.width = W * dpr; canvas.height = H * dpr; canvas.style.height = H + "px";
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      if (world) view = computeView();
    }
    const positioned = () => o.markers.filter((m) => typeof m.lat === "number" && typeof m.lon === "number");

    function draw(now) {
      if (!container.isConnected) { cancelAnimationFrame(raf); raf = 0; return; }
      ctx.clearRect(0, 0, W, H);
      const [fr, fg, fb] = rgb(css("--text-faint")), [ar, ag, ab] = rgb(css("--accent")), [okr, okg, okb] = rgb(css("--ok"));
      const zoomed = view.lon1 - view.lon0 < 200 && hi;
      const ds = zoomed ? hi : world;
      const cell = (W / (view.lon1 - view.lon0)) * (360 / ds.cols), dotR = Math.min(2.6, Math.max(1, cell * (zoomed ? 0.17 : 0.2))), R = 90, R2 = R * R;
      const lonStep = 360 / ds.cols, latStep = (ds.latTop - ds.latBottom) / ds.rows;
      ctx.fillStyle = `rgba(${fr},${fg},${fb},0.42)`;
      const glow = [];
      for (let r = 0; r < ds.rows; r++) {
        const line = ds.data[r];
        const lat = ds.latTop - (r + 0.5) * latStep;
        if (lat > view.lat0 + latStep || lat < view.lat1 - latStep) continue;
        for (let c = 0; c < ds.cols; c++) {
          if (line[c] !== "1") continue;
          const lon = -180 + (c + 0.5) * lonStep;
          if (lon < view.lon0 - lonStep || lon > view.lon1 + lonStep) continue;
          const [x, y] = project(lat, lon), dx = x - mx, dy = y - my, d2 = dx * dx + dy * dy;
          if (d2 < R2) glow.push([x, y, 1 - Math.sqrt(d2) / R]); else if (dotR < 1.5) ctx.fillRect(x - dotR, y - dotR, dotR * 2, dotR * 2); else { ctx.beginPath(); ctx.arc(x, y, dotR, 0, 6.2832); ctx.fill(); }
        }
      }
      for (const [x, y, k] of glow) { ctx.fillStyle = `rgba(${ar},${ag},${ab},${0.35 + k * 0.6})`; ctx.beginPath(); ctx.arc(x, y, dotR * (1 + k * 1.4), 0, 6.2832); ctx.fill(); }

      const pm = {}; for (const m of positioned()) pm[m.id] = project(m.lat, m.lon);
      // Flux de données : courbes avec un paquet qui voyage
      for (const [a, b, live = true] of o.arcs) {
        if (!pm[a] || !pm[b]) continue;
        const [x1, y1] = pm[a], [x2, y2] = pm[b], mxp = (x1 + x2) / 2, myp = (y1 + y2) / 2 - Math.hypot(x2 - x1, y2 - y1) * 0.28;
        ctx.strokeStyle = `rgba(${ar},${ag},${ab},${live ? 0.4 : 0.14})`; ctx.lineWidth = 1.2; ctx.setLineDash([3, 5]);
        ctx.beginPath(); ctx.moveTo(x1, y1); ctx.quadraticCurveTo(mxp, myp, x2, y2); ctx.stroke(); ctx.setLineDash([]);
        if (!live) continue;
        for (let k = 0; k < 2; k++) {
          const t = ((now / 2600 + k * 0.5) % 1), u = 1 - t;
          const px = u * u * x1 + 2 * u * t * mxp + t * t * x2, py = u * u * y1 + 2 * u * t * myp + t * t * y2;
          const g = ctx.createRadialGradient(px, py, 0, px, py, 9); g.addColorStop(0, `rgba(${ar},${ag},${ab},0.9)`); g.addColorStop(1, `rgba(${ar},${ag},${ab},0)`);
          ctx.fillStyle = g; ctx.beginPath(); ctx.arc(px, py, 9, 0, 6.2832); ctx.fill();
        }
      }
      // Repères (les étiquettes se placent à droite, à gauche, au-dessus ou en dessous selon la place libre)
      const taken = [];
      const fs = Math.max(11, W / 90);
      ctx.font = `600 ${fs}px ${css("--font-ui") || "system-ui"}`; ctx.textBaseline = "middle";
      const labelSpot = (m, x, y) => {
        const w = m.label ? ctx.measureText(m.label).width : 0;
        for (const [dx, dy, align] of [[11, -1, "left"], [-11, -1, "right"], [0, -20, "center"], [0, 20, "center"], [11, -22, "left"], [-11, 20, "right"]]) {
          const x0 = align === "left" ? x + dx : align === "right" ? x + dx - w : x - w / 2, r = [x0 - 3, y + dy - fs / 2, w + 6, fs + 2];
          const hit = taken.some((t) => r[0] < t[0] + t[2] && r[0] + r[2] > t[0] && r[1] < t[1] + t[3] && r[1] + r[3] > t[1]);
          if (!hit) { taken.push(r); return [x + dx, y + dy, align]; }
        }
        return [x + 11, y - 1, "left"];
      };
      for (const m of positioned()) {
        const [x, y] = pm[m.id], on = m.online !== false, col = on ? [okr, okg, okb] : [fr, fg, fb];
        const pulse = on ? (Math.sin(now / 520 + x) + 1) / 2 : 0;
        if (on) { ctx.strokeStyle = `rgba(${col[0]},${col[1]},${col[2]},${0.5 - pulse * 0.4})`; ctx.lineWidth = 1.5; ctx.beginPath(); ctx.arc(x, y, 6 + pulse * 9, 0, 6.2832); ctx.stroke(); }
        ctx.fillStyle = m.kind === "local" ? `rgb(${ar},${ag},${ab})` : `rgb(${col[0]},${col[1]},${col[2]})`;
        ctx.beginPath(); ctx.arc(x, y, m.kind === "local" ? 5.5 : 4.5, 0, 6.2832); ctx.fill();
        ctx.strokeStyle = css("--bg"); ctx.lineWidth = 2; ctx.stroke();
        if (m.label) { const [lx, ly, align] = labelSpot(m, x, y); ctx.textAlign = align; ctx.fillStyle = css("--text"); ctx.strokeStyle = css("--bg"); ctx.lineWidth = 3; ctx.strokeText(m.label, lx, ly); ctx.fillText(m.label, lx, ly); ctx.textAlign = "left"; }
      }
      raf = requestAnimationFrame(draw);
    }

    canvas.addEventListener("pointermove", (e) => {
      const b = canvas.getBoundingClientRect(); mx = e.clientX - b.left; my = e.clientY - b.top;
      const hit = positioned().map((m) => ({ m, p: project(m.lat, m.lon) })).find(({ p }) => Math.hypot(p[0] - mx, p[1] - my) < 12);
      if (hit) { tip.innerHTML = `<b>${hit.m.label}</b>${hit.m.sub ? `<br><span style="opacity:.7">${hit.m.sub}</span>` : ""}`; tip.style.left = hit.p[0] + "px"; tip.style.top = hit.p[1] + "px"; tip.style.opacity = 1; }
      else tip.style.opacity = 0;
    });
    canvas.addEventListener("pointerleave", () => { mx = my = -999; tip.style.opacity = 0; });
    const paintFit = () => { fitBtn.innerHTML = fit ? (o.worldLabel || "Monde entier") : (o.fitLabel || "Ajuster"); fitBtn.hidden = !positioned().length; };
    fitBtn.addEventListener("click", () => { fit = !fit; if (world) view = computeView(); paintFit(); });
    const ro = new ResizeObserver(() => { if (world) size(); }); ro.observe(container);

    loadWorld().then((w) => { world = w; size(); paintFit(); raf = requestAnimationFrame(draw); loadHi().then((h) => { hi = h; }).catch(() => {}); }).catch(() => { container.innerHTML = ""; });
    return { update(next) { Object.assign(o, next); } };
  }

  window.WorldMap = { mount, CITIES };
})();
