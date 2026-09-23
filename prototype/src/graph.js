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

      // Reference lap: filled areas, throttle under brake.
      if (ref && sc.refOpacity > 0) {
        const pts = refPoints(ref, fr, -sc.behind - margin, sc.ahead + margin);
        this.fillArea(pts.v, pts.t, RGB.throttle, sc.refOpacity, X, Y, plot);
        this.fillArea(pts.v, pts.b, RGB.brake, sc.refOpacity, X, Y, plot);
      }

      // Live inputs: solid lines up to the car.
      const live = livePoints(sc.live, sc, -sc.behind - margin);
      this.strokeLine(live.v, live.t, RGB.throttle, X, Y);
      this.strokeLine(live.v, live.b, RGB.brake, X, Y);
      ctx.restore();

      // Brake points: a mark at each reference brake-on, your gap to it underlined.
      if (ref && sc.cue) this.drawBrakePoints(sc, ref, fr, X, Y, plot);

      // Car position: cursor, playhead and current-value dots.
      ctx.strokeStyle = "rgba(255, 255, 255, 0.92)";
      ctx.lineWidth = 1.5;
      ctx.beginPath(); ctx.moveTo(cx, plot.y - 2); ctx.lineTo(cx, plot.y + plot.h); ctx.stroke();
      ctx.fillStyle = "#fff";
      ctx.beginPath(); ctx.moveTo(cx - 4, plot.y - 7); ctx.lineTo(cx + 4, plot.y - 7); ctx.lineTo(cx, plot.y - 2); ctx.closePath(); ctx.fill();
      const dots = [];
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
      if (sc.labels.show) this.drawPeakLabels(sc, ref, fr, X, Y, plot, [...sc.obstacles, ...placedX, ...dots]);

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
      ctx.save();
      ctx.shadowColor = `rgba(${rgb}, 0.55)`;
      ctx.shadowBlur = 6;
      ctx.strokeStyle = `rgb(${rgb})`;
      ctx.lineWidth = 2;
      ctx.stroke(p);
      ctx.restore();
    }

    dot(x, y, rgb, hollow) {
      const ctx = this.ctx;
      ctx.fillStyle = `rgb(${INK.surface})`;
      ctx.beginPath(); ctx.arc(x, y, hollow ? 4.5 : 6, 0, Math.PI * 2); ctx.fill();
      ctx.beginPath(); ctx.arc(x, y, hollow ? 2.75 : 4, 0, Math.PI * 2);
      if (hollow) { ctx.strokeStyle = `rgb(${rgb})`; ctx.lineWidth = 1.5; ctx.stroke(); }
      else { ctx.fillStyle = `rgb(${rgb})`; ctx.fill(); }
    }

    drawPeakLabels(sc, ref, fr, X, Y, plot, obstacles) {
      const { ctx } = this;
      const { mode, min } = sc.labels;
      const isDist = sc.axis === "distance";
      const items = [];

      if (mode !== "ref") {
        for (const e of sc.live.events) {
          const v = isDist ? e.peakD - sc.now.D : e.peakT - sc.now.t;
          if (e.peak >= min && v >= -sc.behind && v <= 0) items.push({ kind: "live", v, peak: e.peak });
        }
      }
      if (ref && mode !== "live") {
        for (const z of ref.zones) {
          if (z.peak < min) continue;
          for (let k = Math.floor((fr.center - sc.behind) / fr.period); k <= Math.floor((fr.center + sc.ahead) / fr.period); k++) {
            const v = k * fr.period + fr.at(z.peakIdx) - fr.center;
            if (v >= -sc.behind && v <= sc.ahead) items.push({ kind: "ref", v, peak: z.peak });
          }
        }
      }
      // Live labels win collisions; within a kind, the one nearest the car does.
      items.sort((a, b) => (a.kind === b.kind ? Math.abs(a.v) - Math.abs(b.v) : a.kind === "live" ? -1 : 1));

      // Place every label first, then draw connectors under all pills.
      const placed = obstacles.slice();
      const shown = [];
      const bh = 16;
      for (const it of items) {
        const live = it.kind === "live";
        const text = `${Math.round(it.peak * 100)}%`;
        const font = live ? `700 11px ${FONT}` : `600 10.5px ${FONT}`;
        ctx.font = font;
        const bw = Math.ceil(ctx.measureText(text).width) + 10;
        const px = X(it.v), py = Y(it.peak);
        const xMin = plot.x, xMax = plot.x + plot.w - bw;
        const cands = [
          { x: clamp(px - bw / 2, xMin, xMax), y: py - 7 - bh, at: "above" },
          { x: clamp(px - bw / 2, xMin, xMax), y: py - 10 - 2 * bh, at: "stack" },
          { x: px + 8, y: py - bh / 2, at: "side" },
          { x: px - 8 - bw, y: py - bh / 2, at: "side" },
          { x: clamp(px - bw / 2, xMin, xMax), y: py + 7, at: "below" },
        ];
        const peakBox = { x: px - 5, y: py - 5, w: 10, h: 10 };
        const box = cands.find((c) => {
          const b = { x: c.x, y: c.y, w: bw, h: bh };
          return c.x >= xMin - 0.5 && c.x <= xMax + 0.5 && c.y >= 2 && c.y + bh <= plot.y + plot.h - 1 &&
            !placed.some((p) => overlaps(p, b)) && !overlaps(peakBox, b, 0);
        });
        if (!box) continue;
        placed.push({ x: box.x, y: box.y, w: bw, h: bh }, peakBox);
        shown.push({ live, text, font, bw, px, py, box, color: live ? INK.brakePill : `rgba(${RGB.brake}, 0.9)` });
      }

      for (const { live, bw, px, py, box, color } of shown) {
        const ax = clamp(px, box.x + 4, box.x + bw - 4);
        if (live && box.at === "above" && ax === px) {
          ctx.fillStyle = color;
          ctx.beginPath(); ctx.moveTo(px - 4, box.y + bh - 0.5); ctx.lineTo(px + 4, box.y + bh - 0.5); ctx.lineTo(px, box.y + bh + 4); ctx.closePath(); ctx.fill();
        } else {
          const side = box.at === "side";
          const sx = side ? (px < box.x ? box.x : box.x + bw) : ax;
          const sy = side ? box.y + bh / 2 : box.y + bh <= py ? box.y + bh : box.y;
          ctx.strokeStyle = live ? color : "rgba(255, 255, 255, 0.4)";
          ctx.lineWidth = 1;
          ctx.beginPath(); ctx.moveTo(sx, sy); ctx.lineTo(px, py); ctx.stroke();
        }
        this.dot(px, py, RGB.brake, !live);
      }

      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      for (const { live, text, font, bw, box, color } of shown) {
        ctx.beginPath();
        ctx.roundRect(box.x + 0.5, box.y + 0.5, bw - 1, bh - 1, 4);
        if (live) {
          ctx.fillStyle = color;
          ctx.fill();
        } else {
          ctx.fillStyle = `rgba(${INK.surface}, 0.9)`;
          ctx.fill();
          ctx.strokeStyle = color;
          ctx.lineWidth = 1;
          ctx.stroke();
        }
        ctx.font = font;
        ctx.fillStyle = live ? "#fff" : INK.text;
        ctx.fillText(text, box.x + bw / 2, box.y + bh / 2 + 0.5);
      }
    }
  }

  ITO.InputGraph = InputGraph;
  ITO.GRAPH_COLORS = RGB;
})(typeof window !== "undefined" ? window : globalThis);
