// Brake peak labels that stay off the traces.
//
// The graph rasterises what it drew into a TraceMask (per pixel column: the rows each line
// passes through, and how far down each reference fill reaches). Every label then tries a
// set of spots around its peak and takes the cheapest: crossing a line costs most, covering
// a fill a little, a long leader a little. A label that can't find a spot clear of the
// lines is left out rather than drawn over the data.
//
// Live and reference peaks of the same braking zone share one tag ("68 72": yours solid,
// the reference plain), so a zone costs one label instead of two stacked ones.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  const COST = {
    line: 25, // per pixel column where a trace line passes through the label
    soft: 6, // per column across a hairline (start/finish, brake point markers)
    fill: 1.5, // per column-equivalent of reference fill covered
    leader: 0.15, // per pixel of leader
    cross: 20, // per trace line a leader crosses
    sticky: 40, // bonus for keeping last frame's spot
  };
  const PAD = 2; // clearance kept from lines and other labels
  const GAP = 7; // label to its dot before a leader is drawn

  const overlaps = (a, b, m = 0) =>
    a.x < b.x + b.w + m && b.x < a.x + a.w + m && a.y < b.y + b.h + m && b.y < a.y + a.h + m;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  // ---------------------------------------------------------------- mask

  class TraceMask {
    // width: canvas width in px; base: y of the 0% line (bottom of the fills).
    constructor(width, base) {
      this.n = Math.max(1, Math.ceil(width));
      this.base = base;
      this.lines = []; // { lo, hi, soft }: per-column row span of one line
      this.fills = []; // per-column top of one fill
    }

    column(soft) {
      const lo = new Float32Array(this.n).fill(Infinity);
      const hi = new Float32Array(this.n).fill(-Infinity);
      const c = { lo, hi, soft };
      this.lines.push(c);
      return c;
    }

    // A polyline (x ascending), stroked `half` px either side.
    addLine(xs, ys, half = 1.5, soft = false) {
      const c = this.column(soft);
      for (let i = 1; i < xs.length; i++) {
        let xa = xs[i - 1], xb = xs[i], ya = ys[i - 1], yb = ys[i];
        if (xb < xa) [xa, xb, ya, yb] = [xb, xa, yb, ya];
        const c0 = Math.max(0, Math.floor(xa - half)), c1 = Math.min(this.n - 1, Math.floor(xb + half));
        for (let k = c0; k <= c1; k++) {
          // The segment's rows within this column; all of them when it's under a pixel wide.
          let y0 = ya, y1 = yb;
          if (xb - xa >= 1) {
            const at = (x) => ya + ((yb - ya) * (clamp(x, xa, xb) - xa)) / (xb - xa);
            y0 = at(k);
            y1 = at(k + 1);
          }
          c.lo[k] = Math.min(c.lo[k], Math.min(y0, y1) - half);
          c.hi[k] = Math.max(c.hi[k], Math.max(y0, y1) + half);
        }
      }
    }

    addVLine(x, y0, y1, soft = true) {
      const c = this.column(soft);
      for (let k = Math.max(0, Math.floor(x - 1)); k <= Math.min(this.n - 1, Math.floor(x + 1)); k++) {
        c.lo[k] = y0;
        c.hi[k] = y1;
      }
    }

    // Area between a polyline and the 0% line.
    addFill(xs, ys) {
      const top = new Float32Array(this.n).fill(Infinity);
      for (let i = 1; i < xs.length; i++) {
        const xa = xs[i - 1], xb = xs[i];
        for (let k = Math.max(0, Math.floor(xa)); k <= Math.min(this.n - 1, Math.floor(xb)); k++) {
          const t = xb - xa < 1e-6 ? 0 : (clamp(k + 0.5, xa, xb) - xa) / (xb - xa);
          top[k] = Math.min(top[k], ys[i - 1] + (ys[i] - ys[i - 1]) * t);
        }
      }
      this.fills.push(top);
    }

    // Cost of the lines and fills under rect r.
    cost(r) {
      let hard = 0, soft = 0, fill = 0;
      const k0 = Math.max(0, Math.floor(r.x)), k1 = Math.min(this.n - 1, Math.ceil(r.x + r.w) - 1);
      const y0 = r.y - PAD, y1 = r.y + r.h + PAD;
      for (let k = k0; k <= k1; k++) {
        let h = false, s = false;
        for (const c of this.lines) {
          if (c.lo[k] <= y1 && c.hi[k] >= y0) c.soft ? (s = true) : (h = true);
        }
        if (h) hard++;
        else if (s) soft++;
        let covered = 0;
        for (const top of this.fills) covered = Math.max(covered, Math.min(r.y + r.h, this.base) - Math.max(r.y, top[k]));
        fill += Math.max(0, covered) / r.h;
      }
      return { hard, cost: COST.line * hard + COST.soft * soft + COST.fill * fill };
    }

    // Trace lines a straight leader crosses.
    crossings(ax, ay, bx, by) {
      const len = Math.hypot(bx - ax, by - ay), steps = Math.max(1, Math.ceil(len / 1.5));
      let n = 0;
      const was = new Array(this.lines.length).fill(false);
      for (let s = 0; s <= steps; s++) {
        const x = ax + ((bx - ax) * s) / steps, y = ay + ((by - ay) * s) / steps;
        const k = Math.floor(x);
        if (k < 0 || k >= this.n) continue;
        this.lines.forEach((c, i) => {
          const on = !c.soft && c.lo[k] <= y && c.hi[k] >= y;
          if (on && !was[i]) n++;
          was[i] = on;
        });
      }
      return n;
    }
  }

  // ---------------------------------------------------------------- placement

  // Nearest point of rect r to (x, y).
  const nearest = (r, x, y) => ({ x: clamp(x, r.x, r.x + r.w), y: clamp(y, r.y, r.y + r.h) });

  // Spots to try for a w × h tag over dots `pts` (one, or two for a pair), best-first by bias.
  function candidates(pts, w, h, bounds) {
    const ax = pts.reduce((s, p) => s + p.x, 0) / pts.length;
    const hiY = Math.min(...pts.map((p) => p.y)), loY = Math.max(...pts.map((p) => p.y));
    const left = Math.min(...pts.map((p) => p.x)), right = Math.max(...pts.map((p) => p.x));
    const shifts = [0, -(w / 2 + 5), w / 2 + 5, -(w + 9), w + 9];
    const out = [];
    const add = (id, x, y, bias) => out.push({ id, x: clamp(x, bounds.x0, bounds.x1 - w), y, bias });
    [5, 11, 19, 29, 41, 55, 72].forEach((gap, gi) =>
      shifts.forEach((dx, si) => add(`a${gi}.${si}`, ax - w / 2 + dx, hiY - gap - h, gi * 0.6 + (si ? 1.5 + si * 0.3 : 0))));
    [...shifts, -(1.6 * w + 12), 1.6 * w + 12].forEach((dx, si) => add(`t${si}`, ax - w / 2 + dx, bounds.y0, 3 + si * 0.3));
    [hiY - h / 2, hiY - h - 3].forEach((y, yi) => {
      add(`r${yi}`, right + 7, y, 2 + yi);
      add(`l${yi}`, left - 7 - w, y, 2.5 + yi);
    });
    add("b", ax - w / 2, loY + 6, 7);
    return out;
  }

  // items: [{ key, pts: [{x, y}], w, h, limit, split }] in priority order; `split` is the
  // items to try instead when a pair finds no spot. Returns [{ item, rect }] for the ones
  // that found a spot. `taken` holds obstacles (header, x band,
  // cursor, markers); `memo` maps key → last spot id, and is rewritten. Leaders stay
  // under `maxLeader` px and at most `maxLabels` are placed, so a small graph shows its
  // most important labels instead of a fan of leaders.
  function placePeakLabels({ items, mask, taken, bounds, memo, maxLeader = Infinity, maxLabels = Infinity }) {
    const placed = [];
    const dots = items.flatMap((it) => it.pts.map((p) => ({ x: p.x - 5, y: p.y - 5, w: 10, h: 10 })));
    const blocked = [...taken];
    const seen = new Set();
    const place = (it) => {
      const own = it.pts.map((p) => ({ x: p.x - 5, y: p.y - 5, w: 10, h: 10 }));
      const others = dots.filter((d) => !own.some((o) => o.x === d.x && o.y === d.y));
      let best = null;
      for (const c of candidates(it.pts, it.w, it.h, bounds)) {
        const r = { x: c.x, y: c.y, w: it.w, h: it.h };
        if (r.y < bounds.y0 || r.y + r.h > bounds.y1 || r.x < bounds.x0 - 0.5 || r.x + r.w > bounds.x1 + 0.5) continue;
        if (own.some((d) => overlaps(d, r, 1)) || others.some((d) => overlaps(d, r, 2)) || blocked.some((b) => overlaps(b, r, PAD))) continue;
        const m = mask.cost(r);
        if (m.hard > it.limit) continue;
        let cost = m.cost + c.bias;
        let far = false;
        for (const p of it.pts) {
          const q = nearest(r, p.x, p.y), d = Math.hypot(q.x - p.x, q.y - p.y);
          if (d > maxLeader) far = true;
          if (d > GAP) cost += COST.leader * d + COST.cross * mask.crossings(q.x, q.y, p.x, p.y);
        }
        if (far) continue;
        if (memo.get(it.key) === c.id) cost -= COST.sticky;
        if (!best || cost < best.cost) best = { cost, rect: r, id: c.id };
      }
      seen.add(it.key);
      if (!best) return false;
      memo.set(it.key, best.id);
      blocked.push(best.rect);
      placed.push({ item: it, rect: best.rect });
      return true;
    };
    for (const it of items) {
      if (placed.length >= maxLabels) break;
      if (!place(it) && it.split) for (const s of it.split) if (placed.length < maxLabels) place(s);
    }
    for (const k of [...memo.keys()]) if (!seen.has(k)) memo.delete(k);
    return placed;
  }

  Object.assign(ITO, { TraceMask, placePeakLabels, LABEL_GAP: GAP });
})(typeof window !== "undefined" ? window : globalThis);
