// Canvas renderer for the throttle/brake graph.
// x is "relative to the car": negative = behind (history), positive = ahead (look-ahead),
// in metres (distance axis) or seconds (time axis). y is pedal travel 0–1.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  const RGB = { throttle: "34, 224, 122", brake: "255, 61, 46", you: "127, 209, 255", target: "245, 200, 80" };
  const INK = {
    surface: "7, 9, 10",
    text: "#e8efee",
    muted: "#7f8b89",
    grid: "rgba(255, 255, 255, 0.07)",
    baseline: "rgba(255, 255, 255, 0.16)",
  };
  // Reference peak labels sit in their own row between the header and the plot, so they
  // never cover a trace at 100%. The plot moves down by RAIL_ROOM to make space for it.
  const RAIL_H = 13;
  const RAIL_GAP = 2; // row to the plot's top edge
  const RAIL_ROOM = 6;
  const PIN_H = 12; // your peak's number
  const PIN_TIP = 4; // its pointer
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
      const ref = sc.ref && sc.showRef ? sc.ref : null;
      const labels = sc.labels;
      const railOn = labels.show && ref && labels.mode !== "live";
      const plot = { x: compact ? 26 : 32, y: sc.headerH + 8 + (railOn ? RAIL_ROOM : 0) };
      plot.w = w - plot.x - 10;
      plot.h = h - plot.y - (showX ? 20 : 8);
      if (plot.w < 40 || plot.h < 16) return;

      const isDist = sc.axis === "distance";
      const span = sc.behind + sc.ahead;
      const X = (v) => plot.x + ((v + sc.behind) / span) * plot.w;
      const Y = (p) => plot.y + (1 - p) * plot.h;
      const cx = X(0);
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

      // Reference peaks: a dotted line through each, under your lines.
      const refPeaks = railOn ? this.refPeaks(sc, ref, fr, X, Y, plot) : [];
      this.drawPeakLines(refPeaks, Y, plot, `rgba(${RGB.target}, 0.55)`);

      // Your peaks: a dotted line through each too, in your colour.
      const livePeaks = labels.show && labels.mode !== "ref" ? this.livePeaks(sc, X, Y) : [];
      this.drawPeakLines(livePeaks, Y, plot, `rgba(${RGB.you}, 0.55)`);

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
      for (const [val, rgb] of [[sc.now.throttle, RGB.throttle], [sc.now.brake, RGB.brake]]) this.dot(cx, Y(val), rgb);

      // Reference peak labels in their row, over the cursor as one passes it.
      const rail = this.drawRail(refPeaks);

      // Your peaks: pinned to the apex of your brake line, on top of everything else.
      this.drawPins(livePeaks, plot, [...sc.obstacles, ...rail]);

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
      // A small mark on the baseline at each (the top of the plot is the peak labels' rail).
      ctx.fillStyle = `rgb(${RGB.brake})`;
      for (const x of xs) {
        ctx.beginPath(); ctx.moveTo(x - 3.5, base - 0.5); ctx.lineTo(x + 3.5, base - 0.5); ctx.lineTo(x, base - 6.5); ctx.closePath(); ctx.fill();
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

    // Reference peaks in view, with their label box on the rail. Labels keep their spot:
    // one that would overlap a label nearer the car (the next zone ahead first) nudges
    // aside a few px, and if that isn't enough it's left off; its line still shows.
    refPeaks(sc, ref, fr, X, Y, plot) {
      const { ctx } = this;
      const lo = -sc.behind, hi = sc.ahead, cx = X(0);
      const out = [];
      ref.zones.forEach((z) => {
        if (z.peak < sc.labels.min) return;
        for (let m = Math.floor((fr.center + lo) / fr.period); m <= Math.floor((fr.center + hi) / fr.period); m++) {
          const v = m * fr.period + fr.at(z.peakIdx) - fr.center;
          if (v >= lo && v <= hi) out.push({ x: X(v), y: Y(z.peak), peak: z.peak });
        }
      });
      ctx.font = `600 10px ${FONT}`;
      const top = plot.y - RAIL_GAP - RAIL_H;
      const placed = [];
      const near = (p) => (p.x >= cx ? (p.x - cx) * 0.5 : cx - p.x); // ahead counts double
      for (const p of [...out].sort((a, b) => near(a) - near(b))) {
        p.text = `${Math.round(p.peak * 100)}%`;
        const w = Math.ceil(ctx.measureText(p.text).width) + 8;
        for (const dx of [0, -4, 4, -8, 8]) {
          const box = { x: clamp(p.x - w / 2 + dx, plot.x, plot.x + plot.w - w), y: top, w, h: RAIL_H };
          if (placed.some((q) => overlaps(q, box, 3))) continue;
          placed.push(box);
          p.box = box;
          break;
        }
      }
      return out;
    }

    // Dotted line from the rail down through each reference peak to the 0% line.
    drawPeakLines(peaks, Y, plot, color) {
      if (!peaks.length) return;
      const { ctx } = this;
      ctx.save();
      ctx.strokeStyle = color;
      ctx.lineWidth = 1;
      ctx.lineCap = "round";
      ctx.setLineDash([0.01, 3]);
      for (const p of peaks) {
        const x = Math.round(p.x) + 0.5;
        ctx.beginPath(); ctx.moveTo(x, plot.y); ctx.lineTo(x, Y(0)); ctx.stroke();
      }
      ctx.restore();
    }

    // The peak dots, then each label at the top of its line. Returns the label boxes.
    drawRail(peaks) {
      const { ctx } = this;
      for (const p of peaks) this.dot(p.x, p.y, RGB.target, true);
      ctx.font = `600 10px ${FONT}`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      const boxes = [];
      for (const p of peaks) {
        const b = p.box;
        if (!b) continue;
        ctx.beginPath();
        ctx.roundRect(b.x + 0.5, b.y + 0.5, b.w - 1, b.h - 1, 3);
        ctx.fillStyle = `rgba(${INK.surface}, 0.72)`; // slightly see-through
        ctx.fill();
        ctx.strokeStyle = `rgba(${RGB.target}, 0.8)`;
        ctx.lineWidth = 1;
        ctx.stroke();
        ctx.fillStyle = `rgb(${RGB.target})`;
        ctx.fillText(p.text, b.x + b.w / 2, b.y + b.h / 2 + 0.5);
        boxes.push(b);
      }
      return boxes;
    }

    // Your peaks: a light blue number, no background, over a light blue pointer that touches the apex
    // of your brake line. Both have a dark outline so they read over the traces and fills.
    // It sits above the apex, or below when above would run into a reference label, the
    // header or another pin. Below, it reaches right from the apex: a peak is usually where the brake
    // line tops out, so that's under the line rather than across the rise to it. The one
    // you're braking in is placed first.
    // Your peaks in view (behind the car), nearest the car first.
    livePeaks(sc, X, Y) {
      const isDist = sc.axis === "distance";
      const out = [];
      for (const e of sc.live.events) {
        const v = isDist ? e.peakD - sc.now.D : e.peakT - sc.now.t;
        if (e.peak >= sc.labels.min && v >= -sc.behind && v <= 0) out.push({ x: X(v), y: Y(e.peak), peak: e.peak, v });
      }
      return out.sort((a, b) => b.v - a.v);
    }

    drawPins(pins, plot, taken) {
      const { ctx } = this;
      ctx.font = `700 10.5px ${FONT}`;
      const placed = [...taken];
      const drawn = [];
      for (const p of pins) {
        const text = `${Math.round(p.peak * 100)}%`;
        const w = Math.ceil(ctx.measureText(text).width) + 4;
        const x = clamp(p.x - w / 2, plot.x, plot.x + plot.w - w);
        const above = { x, y: p.y - 2 - PIN_TIP - PIN_H, w, h: PIN_H, up: false };
        const below = { x: clamp(p.x - 9, plot.x, plot.x + plot.w - w), y: p.y + 2 + PIN_TIP, w, h: PIN_H, up: true };
        const clear = (b) => b.y >= 2 && b.y + b.h <= plot.y + plot.h && !placed.some((q) => overlaps(q, b, 1));
        const box = clear(above) ? above : clear(below) ? below : above;
        placed.push(box);
        drawn.push({ box, text, ax: clamp(p.x, box.x + 3, box.x + w - 3), ay: p.y });
      }
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.lineJoin = "round";
      ctx.strokeStyle = `rgba(${INK.surface}, 0.92)`;
      ctx.fillStyle = `rgb(${RGB.you})`;
      for (const { box, text, ax, ay } of drawn) {
        // The pointer's tip sits on the top (or bottom) edge of the line at the apex.
        const edge = box.up ? box.y + 1 : box.y + box.h - 1;
        const tip = box.up ? ay + 2 : ay - 2;
        const arrow = new Path2D();
        arrow.moveTo(ax - 3.5, edge); arrow.lineTo(ax + 3.5, edge); arrow.lineTo(ax, tip); arrow.closePath();
        ctx.lineWidth = 2;
        ctx.stroke(arrow);
        ctx.fill(arrow);
        const tx = box.x + box.w / 2, ty = box.y + box.h / 2 + 0.5;
        ctx.lineWidth = 3.5;
        ctx.strokeText(text, tx, ty);
        ctx.fillText(text, tx, ty);
      }
    }
  }

  ITO.InputGraph = InputGraph;
  ITO.GRAPH_COLORS = RGB;
})(typeof window !== "undefined" ? window : globalThis);
