// Canvas renderer for the throttle/brake graph.
// x is "relative to the car": negative = behind (history), positive = ahead (look-ahead),
// in metres (distance axis) or seconds (time axis). y is pedal travel 0–1.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  const RGB = { throttle: "34, 224, 122", brake: "255, 61, 46" };
  const INK = {
    surface: "7, 9, 10",
    text: "#e8efee",
    muted: "#7f8b89",
    grid: "rgba(255, 255, 255, 0.07)",
    baseline: "rgba(255, 255, 255, 0.16)",
    brakePill: "#e0301f", // a step darker than the line so white text clears 4.5:1
    refText: "#ffb8b0", // reference peak numbers: brake red, lightened to read on the fills
  };
  const FONT = '"Barlow Semi Condensed", "Segoe UI", system-ui, sans-serif';
  const DIST_STEPS = [25, 50, 100, 200, 250, 500, 1000, 2000];
  const TIME_STEPS = [0.5, 1, 2, 5, 10];

  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
  const overlaps = (a, b, m = 2) =>
    a.x < b.x + b.w + m && b.x < a.x + a.w + m && a.y < b.y + b.h + m && b.y < a.y + a.h + m;

  // Where the reference lap sits on the x-axis: one period per lap, centred on the car.
  function refFrame(sc, ref) {
    if (sc.axis === "distance") return { center: sc.now.D, period: sc.L, at: (i) => ref.pct[i] * sc.L };
    const T = ref.lapTime;
    return { center: sc.now.lap * T + ref.indexAtPct(sc.now.pct) / ref.hz, period: T, at: (i) => i / ref.hz };
  }

  // Reference samples in [lo, hi], wrapping across start/finish.
  function refPoints(ref, fr, lo, hi) {
    const v = [], b = [], t = [];
    for (let k = Math.floor((fr.center + lo) / fr.period); k <= Math.floor((fr.center + hi) / fr.period); k++) {
      const base = k * fr.period - fr.center;
      let a = 0, z = ref.n;
      while (a < z) {
        const m = (a + z) >> 1;
        if (base + fr.at(m) < lo) a = m + 1; else z = m;
      }
      for (let i = Math.max(0, a - 1); i < ref.n; i++) {
        const x = base + fr.at(i);
        v.push(x); b.push(ref.brake[i]); t.push(ref.throttle[i]);
        if (x > hi) break;
      }
    }
    return { v, b, t };
  }

  function livePoints(live, sc, lo) {
    const isDist = sc.axis === "distance";
    const arr = isDist ? live.D : live.t;
    const now = isDist ? sc.now.D : sc.now.t;
    let a = live.head, z = arr.length;
    while (a < z) {
      const m = (a + z) >> 1;
      if (arr[m] - now < lo) a = m + 1; else z = m;
    }
    const v = [], b = [], t = [];
    for (let i = Math.max(live.head, a - 1); i < arr.length; i++) {
      v.push(arr[i] - now); b.push(live.b[i]); t.push(live.th[i]);
    }
    return { v, b, t };
  }

  class InputGraph {
    constructor(canvas) {
      this.canvas = canvas;
      this.ctx = canvas.getContext("2d");
      this.w = this.h = 0;
      this.dpr = 1;
      this.labelSpots = new Map(); // label key → last frame's spot, so labels don't hop
      this.labelOffsets = new Map(); // label key → where it was drawn relative to its peak
    }

    resize(w, h) {
      const dpr = root.devicePixelRatio || 1;
      this.w = w; this.h = h; this.dpr = dpr;
      this.canvas.width = Math.max(1, Math.round(w * dpr));
      this.canvas.height = Math.max(1, Math.round(h * dpr));
      this.canvas.style.width = `${w}px`;
      this.canvas.style.height = `${h}px`;
    }

    // sc: { axis, behind, ahead, L, now, live, ref, showRef, refOpacity,
    //       labels: { show, mode, min }, headerH, obstacles: [{x,y,w,h}],
    //       cue: brake point state from BrakeCue.update(), or null }
    render(sc) {
      const { ctx, w, h } = this;
      ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      if (!sc.now) return;

      const compact = w < 400;
      const showX = h >= 118;
      const plot = { x: compact ? 26 : 32, y: sc.headerH + 8 };
      plot.w = w - plot.x - 10;
      plot.h = h - plot.y - (showX ? 20 : 8);
      if (plot.w < 40 || plot.h < 16) return;

      const isDist = sc.axis === "distance";
      const span = sc.behind + sc.ahead;
      const X = (v) => plot.x + ((v + sc.behind) / span) * plot.w;
      const Y = (p) => plot.y + (1 - p) * plot.h;
      const cx = X(0);
      const ref = sc.ref && sc.showRef ? sc.ref : null;
      const fr = ref ? refFrame(sc, ref) : null;
      const margin = span * 0.02;

      // Grid: 0 / 50 / 100, solid hairlines.
      ctx.lineWidth = 1;
      for (const p of [1, 0.5, 0]) {
        const y = Math.round(Y(p)) + 0.5;
        ctx.strokeStyle = p === 0 ? INK.baseline : INK.grid;
        ctx.beginPath(); ctx.moveTo(plot.x, y); ctx.lineTo(plot.x + plot.w, y); ctx.stroke();
      }

      // Start/finish line.
      const sfLines = [];
      if (isDist) {
        for (let k = Math.ceil((sc.now.D - sc.behind) / sc.L); k * sc.L <= sc.now.D + sc.ahead; k++) sfLines.push(k * sc.L - sc.now.D);
        ctx.strokeStyle = "rgba(255, 255, 255, 0.22)";
        for (const v of sfLines) {
          const x = Math.round(X(v)) + 0.5;
          ctx.beginPath(); ctx.moveTo(x, plot.y); ctx.lineTo(x, plot.y + plot.h); ctx.stroke();
        }
      }

      ctx.save();
      ctx.beginPath();
      ctx.rect(plot.x, plot.y - 4, plot.w, plot.h + 8);
      ctx.clip();

      // What's drawn, column by column, for the peak labels to keep off.
      const mask = new ITO.TraceMask(w, Y(0));
      const onScreen = (vs, ys) => [vs.map(X), ys.map(Y)];

      // Reference lap: filled areas, throttle under brake.
      if (ref && sc.refOpacity > 0) {
        const pts = refPoints(ref, fr, -sc.behind - margin, sc.ahead + margin);
        this.fillArea(pts.v, pts.t, RGB.throttle, sc.refOpacity, X, Y, plot);
        this.fillArea(pts.v, pts.b, RGB.brake, sc.refOpacity, X, Y, plot);
        for (const ys of [pts.t, pts.b]) {
          const [xs, yy] = onScreen(pts.v, ys);
          mask.addLine(xs, yy, 1);
          mask.addFill(xs, yy);
        }
      }

      // Live inputs: solid lines up to the car.
      const live = livePoints(sc.live, sc, -sc.behind - margin);
      this.strokeLine(live.v, live.t, RGB.throttle, X, Y);
      this.strokeLine(live.v, live.b, RGB.brake, X, Y);
      ctx.restore();
      for (const ys of [live.t, live.b]) mask.addLine(...onScreen(live.v, ys), 2);
      for (const v of sfLines) mask.addVLine(X(v), plot.y, plot.y + plot.h);

      // Brake points: a mark at each reference brake-on, your gap to it underlined.
      const markers = [];
      if (ref && sc.cue) {
        for (const x of this.drawBrakePoints(sc, ref, fr, X, Y, plot)) {
          mask.addVLine(x, plot.y + 3, plot.y + plot.h);
          markers.push({ x: x - 4.5, y: plot.y - 2, w: 9, h: 7 });
        }
      }

      // Car position: cursor, playhead and current-value dots.
      ctx.strokeStyle = "rgba(255, 255, 255, 0.92)";
      ctx.lineWidth = 1.5;
      ctx.beginPath(); ctx.moveTo(cx, plot.y - 2); ctx.lineTo(cx, plot.y + plot.h); ctx.stroke();
      ctx.fillStyle = "#fff";
      ctx.beginPath(); ctx.moveTo(cx - 4, plot.y - 7); ctx.lineTo(cx + 4, plot.y - 7); ctx.lineTo(cx, plot.y - 2); ctx.closePath(); ctx.fill();
      // Labels keep off the cursor and its dots: that's where you're looking.
      const dots = [{ x: cx - 4, y: plot.y - 8, w: 8, h: plot.h + 8 }];
      for (const [val, rgb] of [[sc.now.throttle, RGB.throttle], [sc.now.brake, RGB.brake]]) {
        const y = Y(val);
        this.dot(cx, y, rgb);
        dots.push({ x: cx - 6, y: y - 6, w: 12, h: 12 });
      }

      // Y labels.
      ctx.font = `600 10px ${FONT}`;
      ctx.fillStyle = INK.muted;
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      for (const p of plot.h >= 56 ? [1, 0.5, 0] : [1, 0]) ctx.fillText(String(p * 100), plot.x - 6, Y(p) + 0.5);

      // X band: car-position pill, then ticks that don't collide with it.
      const placedX = [];
      if (showX) {
        const bandY = plot.y + plot.h + 4;
        const lapDist = ((sc.now.D % sc.L) + sc.L) % sc.L;
        ctx.font = `700 10px ${FONT}`;
        const pillText = `${Math.round(lapDist)}m`;
        const pw = Math.ceil(ctx.measureText(pillText).width) + 10;
        const pill = { x: clamp(cx - pw / 2, plot.x - 4, w - pw - 2), y: bandY - 1, w: pw, h: 14 };
        ctx.fillStyle = "#e8efee";
        ctx.beginPath(); ctx.roundRect(pill.x, pill.y, pill.w, pill.h, 3); ctx.fill();
        ctx.fillStyle = `rgb(${INK.surface})`;
        ctx.textAlign = "center";
        ctx.fillText(pillText, pill.x + pill.w / 2, pill.y + pill.h / 2 + 0.5);
        placedX.push(pill);

        const ticks = [];
        const pxPer = plot.w / span;
        if (isDist) {
          for (const v of sfLines) ticks.push({ v, text: "S/F", strong: true });
          const step = DIST_STEPS.find((s) => s * pxPer >= (compact ? 70 : 90)) || 2000;
          const lo = sc.now.D - sc.behind, hi = sc.now.D + sc.ahead;
          for (let k = Math.floor(lo / sc.L); k <= Math.floor(hi / sc.L); k++) {
            for (let d = step; d < sc.L; d += step) {
              const D = k * sc.L + d;
              if (D >= lo && D <= hi) ticks.push({ v: D - sc.now.D, text: `${d}m` });
            }
          }
        } else {
          const step = TIME_STEPS.find((s) => s * pxPer >= (compact ? 56 : 70)) || 10;
          for (let v = Math.ceil(-sc.behind / step) * step; v <= sc.ahead + 1e-9; v += step) {
            if (Math.abs(v) < 1e-9) continue;
            ticks.push({ v, text: `${v > 0 ? "+" : "−"}${+Math.abs(v).toFixed(1)}s` });
          }
        }
        ctx.font = `600 10px ${FONT}`;
        for (const t of ticks) {
          const tw = ctx.measureText(t.text).width;
          const x = X(t.v);
          const box = { x: x - tw / 2 - 2, y: bandY, w: tw + 4, h: 12 };
          if (box.x < plot.x - 6 || box.x + box.w > w - 2 || placedX.some((p) => overlaps(p, box, 3))) continue;
          placedX.push(box);
          ctx.fillStyle = t.strong ? INK.text : INK.muted;
          ctx.fillText(t.text, x, bandY + 6.5);
        }
      }

      // Brake peak labels.
      if (sc.labels.show) this.drawPeakLabels(sc, ref, fr, X, Y, plot, mask, [...sc.obstacles, ...placedX, ...dots, ...markers]);

      // Empty state.
      if (!sc.ref) {
        const x0 = sc.ahead > 0 && plot.x + plot.w - cx > 150 ? cx : plot.x;
        const mid = (x0 + plot.x + plot.w) / 2;
        if (plot.x + plot.w - x0 > 150) {
          ctx.textAlign = "center";
          ctx.fillStyle = INK.text;
          ctx.font = `700 11px ${FONT}`;
          ctx.fillText("NO REFERENCE LAP", mid, plot.y + plot.h / 2 - 7);
          ctx.fillStyle = INK.muted;
          ctx.font = `500 10.5px ${FONT}`;
          ctx.fillText("Drop a Garage 61 CSV here, or ⚙ → Reference", mid, plot.y + plot.h / 2 + 8);
        }
      }
    }

    drawBrakePoints(sc, ref, fr, X, Y, plot) {
      const { ctx } = this;
      const lo = -sc.behind, hi = sc.ahead;
      const refX = (m, k) => m * fr.period + fr.at(ref.zones[k].start) - fr.center;
      const base = Math.round(Y(0)) + 0.5;

      const xs = [];
      for (let m = Math.floor((fr.center + lo) / fr.period); m <= Math.floor((fr.center + hi) / fr.period); m++) {
        for (const k of sc.cue.cueZones) {
          const v = refX(m, k);
          if (v >= lo && v <= hi) xs.push(Math.round(X(v)) + 0.5);
        }
      }
      ctx.save();
      ctx.setLineDash([2, 3]);
      ctx.lineWidth = 1;
      ctx.strokeStyle = `rgba(${RGB.brake}, 0.5)`;
      for (const x of xs) { ctx.beginPath(); ctx.moveTo(x, plot.y + 3); ctx.lineTo(x, base); ctx.stroke(); }
      ctx.restore();
      ctx.fillStyle = `rgb(${RGB.brake})`;
      for (const x of xs) {
        ctx.beginPath(); ctx.moveTo(x - 3.5, plot.y - 1); ctx.lineTo(x + 3.5, plot.y - 1); ctx.lineTo(x, plot.y + 3.5); ctx.closePath(); ctx.fill();
      }

      // Under the baseline, from the reference brake-on to yours, in the grade's colour.
      const y = base + 2;
      for (const mk of sc.cue.marks) {
        const v0 = refX(mk.m, mk.k);
        const v1 = sc.axis === "distance" ? mk.onD - sc.now.D : mk.onT - sc.now.t;
        if (Math.max(v0, v1) < lo || Math.min(v0, v1) > hi) continue;
        const x0 = X(v0), x1 = X(v1);
        ctx.strokeStyle = `rgb(${ITO.GRADES[mk.grade].rgb})`;
        ctx.lineCap = "butt";
        ctx.lineWidth = 2.5;
        ctx.beginPath(); ctx.moveTo(x0, y); ctx.lineTo(x1, y); ctx.stroke();
        ctx.lineWidth = 1.5;
        ctx.beginPath(); ctx.moveTo(x1, y - 4); ctx.lineTo(x1, y + 2); ctx.stroke();
      }
      return xs;
    }

    fillArea(v, val, rgb, op, X, Y, plot) {
      if (v.length < 2) return;
      const ctx = this.ctx;
      const base = plot.y + plot.h;
      const area = new Path2D();
      const edge = new Path2D();
      area.moveTo(X(v[0]), base);
      let pen = false;
      for (let i = 0; i < v.length; i++) {
        const x = X(v[i]), y = Y(val[i]);
        area.lineTo(x, y);
        // Edge only where the trace is off the floor, so zero stretches stay clean.
        const on = val[i] > 0.004 || (i > 0 && val[i - 1] > 0.004) || (i < v.length - 1 && val[i + 1] > 0.004);
        if (on) { pen ? edge.lineTo(x, y) : edge.moveTo(x, y); pen = true; } else pen = false;
      }
      area.lineTo(X(v[v.length - 1]), base);
      area.closePath();
      const g = this.ctx.createLinearGradient(0, plot.y, 0, base);
      g.addColorStop(0, `rgba(${rgb}, ${0.36 * op})`);
      g.addColorStop(1, `rgba(${rgb}, ${0.02 * op})`);
      ctx.fillStyle = g;
      ctx.fill(area);
      ctx.strokeStyle = `rgba(${rgb}, ${0.6 * op})`;
      ctx.lineWidth = 1.25;
      ctx.lineJoin = "round";
      ctx.stroke(edge);
    }

    strokeLine(v, val, rgb, X, Y) {
      if (v.length < 2) return;
      const ctx = this.ctx;
      const p = new Path2D();
      for (let i = 0; i < v.length; i++) i ? p.lineTo(X(v[i]), Y(val[i])) : p.moveTo(X(v[i]), Y(val[i]));
      ctx.lineJoin = "round";
      ctx.lineCap = "round";
      ctx.strokeStyle = `rgba(${INK.surface}, 0.55)`; // keeps the line legible over its own fill
      ctx.lineWidth = 4;
      ctx.stroke(p);
      ctx.strokeStyle = `rgb(${rgb})`;
      ctx.lineWidth = 2;
      ctx.stroke(p);
    }

    dot(x, y, rgb, hollow) {
      const ctx = this.ctx;
      ctx.fillStyle = `rgb(${INK.surface})`;
      ctx.beginPath(); ctx.arc(x, y, hollow ? 4.5 : 6, 0, Math.PI * 2); ctx.fill();
      ctx.beginPath(); ctx.arc(x, y, hollow ? 2.75 : 4, 0, Math.PI * 2);
      if (hollow) { ctx.strokeStyle = `rgb(${rgb})`; ctx.lineWidth = 1.5; ctx.stroke(); }
      else { ctx.fillStyle = `rgb(${rgb})`; ctx.fill(); }
    }

    // Brake peak labels: one tag per braking zone where both your peak and the reference's
    // are shown, placed clear of the traces (see peak-labels.js), else left out.
    drawPeakLabels(sc, ref, fr, X, Y, plot, mask, obstacles) {
      const { ctx } = this;
      const { mode, min } = sc.labels;
      const isDist = sc.axis === "distance";
      const lo = -sc.behind, hi = sc.ahead;
      const cx = X(0);

      // Peaks in view.
      const lives = [];
      if (mode !== "ref") {
        for (const e of sc.live.events) {
          const v = isDist ? e.peakD - sc.now.D : e.peakT - sc.now.t;
          if (e.peak >= min && v >= lo && v <= 0) {
            lives.push({ key: `L${Math.round((e.onT ?? e.peakT) * 60)}`, v, peak: e.peak, active: e.active });
          }
        }
      }
      const refs = [];
      if (ref && mode !== "live") {
        ref.zones.forEach((z, k) => {
          if (z.peak < min) return;
          for (let m = Math.floor((fr.center + lo) / fr.period); m <= Math.floor((fr.center + hi) / fr.period); m++) {
            const base = m * fr.period - fr.center;
            const v = base + fr.at(z.peakIdx);
            if (v >= lo && v <= hi) refs.push({ key: `R${m}:${k}`, v, peak: z.peak, from: base + fr.at(z.start), to: base + fr.at(z.end) });
          }
        });
      }

      // Pair each of your peaks with the reference zone it falls in.
      const paired = new Set();
      const tags = [];
      for (const l of lives) {
        const r = refs
          .filter((r) => !paired.has(r) && l.v >= r.from - 0.3 * (r.to - r.from) && l.v <= r.to + 0.3 * (r.to - r.from))
          .sort((a, b) => Math.abs(a.v - l.v) - Math.abs(b.v - l.v))[0];
        if (r && Math.abs(X(r.v) - X(l.v)) <= 70) {
          paired.add(r);
          tags.push({ key: `P${l.key}`, live: l, ref: r });
        } else tags.push({ key: l.key, live: l });
      }
      for (const r of refs) if (!paired.has(r)) tags.push({ key: r.key, ref: r });

      // Priority: the zone you're braking in, the next one ahead, then nearest the car, with
      // big peaks ahead of light dabs.
      const next = refs.filter((r) => r.v > 0).sort((a, b) => a.v - b.v)[0];
      const rank = (t) => (t.live && t.live.active ? 0 : t.ref === next && !t.live ? 1 : 2);
      const peakOf = (t) => Math.max(t.live ? t.live.peak : 0, t.ref ? t.ref.peak : 0);
      const at = (t) => Math.abs(X((t.live || t.ref).v) - cx) - 150 * peakOf(t);
      tags.sort((a, b) => rank(a) - rank(b) || at(a) - at(b));

      // Sizes.
      const LIVE_FONT = `700 10.5px ${FONT}`, REF_FONT = `600 10.5px ${FONT}`;
      const text = (p) => String(Math.round(p * 100));
      const item = (t) => {
        ctx.font = LIVE_FONT;
        const lw = t.live ? Math.ceil(ctx.measureText(text(t.live.peak)).width) + 8 : 0;
        ctx.font = REF_FONT;
        const rw = t.ref ? Math.ceil(ctx.measureText(text(t.ref.peak)).width) + (t.live ? 4 : 2) : 0;
        const pts = [t.live, t.ref].filter(Boolean).map((p) => ({ x: X(p.v), y: Y(p.peak) }));
        // A reference label may graze a line by a column or two; yours, a quarter of its width.
        const wTot = lw + (lw && rw ? 2 : 0) + rw;
        return { key: t.key, tag: t, pts, w: wTot, h: t.live ? 14 : 12, lw, rw, limit: t.live ? Math.floor(wTot / 4) : 1 };
      };
      // A pair that doesn't fit may still fit as two separate labels.
      const items = tags.map((t) => ({
        ...item(t),
        split: t.live && t.ref ? [item({ key: t.live.key, live: t.live }), item({ key: t.ref.key, ref: t.ref })] : null,
      }));

      const placed = ITO.placePeakLabels({
        items,
        mask,
        taken: obstacles,
        bounds: { x0: plot.x, x1: plot.x + plot.w, y0: 2, y1: plot.y + plot.h - 1 },
        memo: this.labelSpots,
        maxLeader: Math.max(22, plot.h * 0.45),
        maxLabels: Math.max(1, Math.floor(plot.w / 80)),
      });

      // When a label changes spot it glides there (about 0.1 s), relative to its peak so it
      // still scrolls with the graph.
      const offsets = new Map();
      for (const p of placed) {
        const a = p.item.pts[0], key = p.item.key;
        const want = { x: p.rect.x - a.x, y: p.rect.y - a.y };
        const was = this.labelOffsets.get(key);
        const cur = was && Math.hypot(want.x - was.x, want.y - was.y) < 90
          ? { x: was.x + (want.x - was.x) * 0.3, y: was.y + (want.y - was.y) * 0.3 }
          : want;
        if (Math.hypot(want.x - cur.x, want.y - cur.y) < 0.5) Object.assign(cur, want);
        offsets.set(key, cur);
        p.rect = { ...p.rect, x: a.x + cur.x, y: a.y + cur.y };
      }
      this.labelOffsets = offsets;

      // Dots and leaders first, then the tags on top.
      for (const { item, rect } of placed) {
        const t = item.tag;
        const parts = [];
        if (t.live) parts.push({ p: t.live, x0: rect.x, x1: rect.x + item.lw, live: true });
        if (t.ref) parts.push({ p: t.ref, x0: rect.x + rect.w - item.rw, x1: rect.x + rect.w, live: false });
        for (const part of parts) {
          const px = X(part.p.v), py = Y(part.p.peak);
          const sub = { x: part.x0, y: rect.y, w: part.x1 - part.x0, h: rect.h };
          const qx = clamp(px, sub.x, sub.x + sub.w), qy = clamp(py, sub.y, sub.y + sub.h);
          const d = Math.hypot(qx - px, qy - py);
          if (d > ITO.LABEL_GAP) {
            const k = (d - (part.live ? 5 : 4)) / d; // stop at the dot's edge
            ctx.strokeStyle = part.live ? `rgba(${RGB.brake}, 0.75)` : "rgba(255, 255, 255, 0.38)";
            ctx.lineWidth = 1;
            ctx.beginPath(); ctx.moveTo(qx, qy); ctx.lineTo(qx + (px - qx) * k, qy + (py - qy) * k); ctx.stroke();
          }
          this.dot(px, py, RGB.brake, !part.live);
        }
      }

      ctx.textBaseline = "middle";
      for (const { item, rect } of placed) {
        const t = item.tag;
        const cy = rect.y + rect.h / 2 + 0.5;
        if (t.live) {
          ctx.fillStyle = INK.brakePill;
          ctx.beginPath(); ctx.roundRect(rect.x, rect.y, item.lw, rect.h, 3); ctx.fill();
          ctx.font = LIVE_FONT;
          ctx.textAlign = "center";
          ctx.fillStyle = "#fff";
          ctx.fillText(text(t.live.peak), rect.x + item.lw / 2, cy);
        }
        if (t.ref) {
          // No box: the number with a dark halo covers only its own strokes.
          ctx.font = REF_FONT;
          ctx.textAlign = "right";
          const x = rect.x + rect.w - 1;
          ctx.lineJoin = "round";
          ctx.lineWidth = 3.5;
          ctx.strokeStyle = `rgba(${INK.surface}, 0.9)`;
          ctx.strokeText(text(t.ref.peak), x, cy);
          ctx.fillStyle = INK.refText;
          ctx.fillText(text(t.ref.peak), x, cy);
        }
      }
    }
  }

  ITO.InputGraph = InputGraph;
  ITO.GRAPH_COLORS = RGB;
})(typeof window !== "undefined" ? window : globalThis);
