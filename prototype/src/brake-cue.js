// Brake point countdown: counts 3-2-1-BRAKE into each of the reference lap's brake
// points, shows that zone's target peak, and grades when your brake went on.
//
// Everything runs on the reference lap's clock (sample index ÷ Hz), which needs only
// LapDistPct from the sim. If you're slower down the straight the counts stretch, but
// BRAKE always lands on the reference brake point.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  // Early to late. "Perfect" is within ±perfect of the reference brake point, "Good" within
  // ±tol, and "very" is past 3 × tol. Chevrons point the way the graph does (early = behind
  // the car = left), so the grade reads without colour.
  const GRADES = {
    veryEarly: { label: "Very early", chip: "«« Early", rgb: "91, 140, 255" },
    early: { label: "Early", chip: "« Early", rgb: "111, 193, 255" },
    good: { label: "Good", chip: "Good", rgb: "46, 230, 160" },
    perfect: { label: "Perfect", chip: "Perfect", rgb: "200, 36, 208", ink: "#fff" }, // "fastest lap" purple
    late: { label: "Late", chip: "Late »", rgb: "255, 177, 59" },
    veryLate: { label: "Very late", chip: "Late »»", rgb: "255, 107, 61" },
    none: { label: "No brake", chip: "No brake", rgb: "127, 139, 137" },
  };
  const VERY = 3;
  const FLASH_HOLD = 0.8; // s the bar shows your grade's colour after you brake…
  const FLASH_DRAIN = 0.3; // …then fades out over this
  const FINAL_HOLD = 3; // s the compact bar shows your final pressure after you release
  const MATCH_BEFORE = 4; // s: a brake-on this long before a brake point still counts as that zone's
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  function gradeOf(dt, tol, perfect = 0) {
    if (dt == null) return "none";
    const a = Math.abs(dt);
    if (a <= perfect) return "perfect";
    if (a <= tol) return "good";
    if (a <= VERY * tol) return dt < 0 ? "early" : "late";
    return dt < 0 ? "veryEarly" : "veryLate";
  }

  // ---------------------------------------------------------------- model

  class BrakeCue {
    constructor() {
      this.setReference(null, 0);
    }

    setReference(ref, trackLength) {
      this.ref = ref;
      this.L = trackLength;
      this.results = new Map(); // "lap:zone" → { key, m, k, dt, dm, event }
      this.order = []; // result keys, oldest first
      this.seen = new WeakSet(); // live brake events already looked at
      this.startA = null;
    }

    // Reference clock: seconds into the reference lap, plus one lap time per lap.
    clock(lap, pct) {
      return lap * this.ref.lapTime + this.ref.indexAtPct(pct) / this.ref.hz;
    }

    // Zone k on lap m, with its brake-on (s) and brake-off (e) on the reference clock.
    occ(m, k) {
      const { ref } = this;
      const z = ref.zones[k];
      return { m, k, key: `${m}:${k}`, s: m * ref.lapTime + z.start / ref.hz, e: m * ref.lapTime + z.end / ref.hz };
    }

    // Brake-off of the zone before (m, k), among the sorted zone indices in `list`.
    prevEnd(m, k, list) {
      const i = list.indexOf(k);
      return i > 0 ? this.occ(m, list[i - 1]).e : this.occ(m - 1, list[list.length - 1]).e;
    }

    // The zone a brake-on at clock `a` belongs to: the first one not yet over, if `a` is
    // after the previous zone ended and no more than MATCH_BEFORE ahead of its brake point.
    // All zones take part, so a light dab the cue skips doesn't count against the next one.
    match(a) {
      const all = this.allIdx;
      const m0 = Math.floor(a / this.ref.lapTime);
      for (let m = m0 - 1; m <= m0 + 1; m++) {
        for (const k of all) {
          const o = this.occ(m, k);
          if (a > o.e) continue;
          return a >= Math.max(this.prevEnd(m, k, all), o.s - MATCH_BEFORE) ? o : null;
        }
      }
      return null;
    }

    record(o, r) {
      this.results.set(o.key, { key: o.key, m: o.m, k: o.k, ...r });
      this.order.push(o.key);
      while (this.order.length > 120) this.results.delete(this.order.shift());
    }

    // cfg: { lead, early, min, tol, perfect }: countdown seconds, cue-early seconds, minimum
    // reference peak (0–1), and the ± "good" and "perfect" windows in seconds.
    update(now, live, cfg) {
      const { ref } = this;
      if (!ref) return { mode: "noref" };
      const zones = ref.zones;
      this.allIdx = zones.map((_, k) => k);
      const cueIdx = this.allIdx.filter((k) => zones[k].peak >= cfg.min);
      if (!cueIdx.length || !now) return { mode: "nozones" };
      const cueSet = new Set(cueIdx);
      const T = ref.lapTime;
      const A = this.clock(now.lap, now.pct);
      if (this.startA == null) this.startA = A;

      // 1. Grade new brake-ons against their zone's reference brake point.
      for (const ev of live.events) {
        if (this.seen.has(ev)) continue;
        this.seen.add(ev);
        if (ev.onLap == null) continue;
        const a = this.clock(ev.onLap, ev.onPct);
        const o = this.match(a);
        if (!o || !cueSet.has(o.k) || this.results.has(o.key)) continue;
        const dm = (ev.onLap + ev.onPct - (o.m + ref.pct[zones[o.k].start])) * this.L;
        this.record(o, { dt: a - o.s, dm, event: ev });
      }

      // 2. Zones driven through without braking.
      const mA = Math.floor(A / T);
      for (let m = mA - 1; m <= mA; m++) {
        for (const k of cueIdx) {
          const o = this.occ(m, k);
          if (o.e < A && o.s >= this.startA && !this.results.has(o.key)) this.record(o, { dt: null, dm: null, event: null });
        }
      }

      // 3. The zone ahead, or the one you're still braking in.
      let cur = null;
      for (let m = mA - 1; m <= mA + 1 && !cur; m++) {
        for (const k of cueIdx) {
          const o = this.occ(m, k);
          const r = this.results.get(o.key);
          // Still braking, or the grade still showing, keeps a zone current.
          if (r ? r.event && (r.event.active || now.t - r.event.onT < FLASH_HOLD + FLASH_DRAIN) : A <= o.e) { cur = o; break; }
        }
      }
      const z = zones[cur.k];
      const r = this.results.get(cur.key);
      const cueAt = cur.s - cfg.early;
      const armAt = cueAt - cfg.lead;
      // After a short straight the count joins part-way (e.g. at 2), never mid-zone.
      const joinAt = Math.max(armAt, this.prevEnd(cur.m, cur.k, cueIdx));
      const dist = (cur.m + ref.pct[z.start] - (now.lap + now.pct)) * this.L;

      const st = {
        mode: "idle",
        beat: 0,
        fill: 0,
        join: clamp((joinAt - armAt) / cfg.lead, 0, 1),
        zoneNo: cueIdx.indexOf(cur.k) + 1,
        zoneCount: cueIdx.length,
        dist,
        target: z.peak,
        live: now.brake,
        peak: null,
        flash: null, // { grade, alpha }: the bar in your grade's colour, just after you brake
        final: null, // { peak, target, age }: your last zone's peak, for FINAL_HOLD s after release
        verdict: null,
        pips: null,
        cueZones: cueIdx,
        marks: [],
      };
      if (r) {
        // Braking: the bar stops where the brake went on.
        st.mode = "braking";
        st.fill = Math.max(st.join, clamp((cur.s + r.dt - armAt) / cfg.lead, 0, 1));
        st.peak = r.event.peak;
        const since = now.t - r.event.onT;
        if (since < FLASH_HOLD + FLASH_DRAIN) {
          st.flash = { grade: gradeOf(r.dt, cfg.tol, cfg.perfect), alpha: since < FLASH_HOLD ? 1 : 1 - (since - FLASH_HOLD) / FLASH_DRAIN };
        }
      } else if (A < joinAt) {
        st.mode = "idle";
      } else if (A < cueAt) {
        st.mode = "countdown";
        st.fill = clamp((A - armAt) / cfg.lead, 0, 1);
        st.beat = clamp(Math.ceil(((cueAt - A) / cfg.lead) * 3), 1, 3);
      } else {
        st.mode = "brake";
        st.fill = 1;
      }

      // Timing: this zone while it's being decided, else the last one graded.
      let v = null;
      if (st.mode === "brake" && A > cur.s) v = { key: cur.key, k: cur.k, dt: A - cur.s, dm: -dist, event: null, pending: true };
      else if (r) v = r;
      else if (this.order.length) v = this.results.get(this.order[this.order.length - 1]);
      if (v) {
        st.verdict = {
          zoneNo: cueIdx.indexOf(v.k) + 1,
          grade: gradeOf(v.dt, cfg.tol, cfg.perfect),
          dt: v.dt,
          dm: v.dm,
          pending: !!v.pending,
          current: v.key === cur.key,
          peak: v.event ? v.event.peak : null,
          target: zones[v.k].peak,
        };
      }

      // Your final pressure in the zone you just left: for FINAL_HOLD s after you release,
      // and only until the next countdown starts.
      const lastKey = this.order[this.order.length - 1];
      const last = lastKey && this.results.get(lastKey);
      if (last && last.event && !last.event.active && last.event.offT != null && st.mode === "idle") {
        const age = now.t - last.event.offT;
        if (age <= FINAL_HOLD) st.final = { peak: last.event.peak, target: zones[last.k].peak, age };
      }

      // One pip per zone: this lap's grade, else last lap's (dimmed).
      st.pips = cueIdx.map((k) => {
        const now0 = this.results.get(`${cur.m}:${k}`);
        const was = now0 || this.results.get(`${cur.m - 1}:${k}`);
        return { grade: was ? gradeOf(was.dt, cfg.tol, cfg.perfect) : null, stale: !now0, current: k === cur.k };
      });

      // Brake-ons to underline on the graph.
      for (let i = Math.max(0, this.order.length - 24); i < this.order.length; i++) {
        const x = this.results.get(this.order[i]);
        if (x && x.event) st.marks.push({ m: x.m, k: x.k, grade: gradeOf(x.dt, cfg.tol, cfg.perfect), onD: x.event.onD, onT: x.event.onT });
      }
      return st;
    }

    // Reference clock one second before the next countdown starts (for the prototype's
    // "next zone" button).
    nextArmAt(now, cfg) {
      const { ref } = this;
      if (!ref || !now) return null;
      const cueIdx = ref.zones.map((_, k) => k).filter((k) => ref.zones[k].peak >= cfg.min);
      if (!cueIdx.length) return null;
      const A = this.clock(now.lap, now.pct);
      for (let m = Math.floor(A / ref.lapTime); ; m++) {
        for (const k of cueIdx) {
          const arm = Math.max(this.occ(m, k).s - cfg.early - cfg.lead, this.prevEnd(m, k, cueIdx));
          if (arm - 1 > A + 0.25) return arm - 1;
        }
      }
    }
  }

  // ---------------------------------------------------------------- view

  const GEAR = '<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M9.97 4.68 L10.27 1.95 L13.73 1.95 L14.03 4.68 L15.75 5.39 L17.89 3.67 L20.33 6.11 L18.61 8.25 L19.32 9.97 L22.05 10.27 L22.05 13.73 L19.32 14.03 L18.61 15.75 L20.33 17.89 L17.89 20.33 L15.75 18.61 L14.03 19.32 L13.73 22.05 L10.27 22.05 L9.97 19.32 L8.25 18.61 L6.11 20.33 L3.67 17.89 L5.39 15.75 L4.68 14.03 L1.95 13.73 L1.95 10.27 L4.68 9.97 L5.39 8.25 L3.67 6.11 L6.11 3.67 L8.25 5.39Z"/><circle cx="12" cy="12" r="3.2"/></svg>';
  const CLOSE = '<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18"/></svg>';
  const COLLAPSE = '<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7"/></svg>';
  const EXPAND = '<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7"/></svg>';
  const TEMPLATE = `
    <header class="ito-header cue-head">
      <span class="ito-grip" aria-hidden="true"></span>
      <h2 class="ito-title">Brake point</h2>
      <span class="cue-next"></span>
      <span class="cue-pips" aria-hidden="true"></span>
      <span class="cue-tools">
        <button class="ito-gear cue-collapse" type="button" aria-label="Collapse to the bar" title="Collapse to the bar">${COLLAPSE}</button>
        <button class="ito-gear cue-gear" type="button" aria-label="Brake point settings" aria-haspopup="dialog" aria-expanded="false">${GEAR}</button>
        <button class="ito-gear cue-close" type="button" aria-label="Hide the brake point countdown">${CLOSE}</button>
      </span>
    </header>
    <div class="cue-body">
      <div class="cue-main">
        <div class="cue-track">
          <i class="cue-join"></i><i class="cue-fill"></i>
          <i class="cue-tick" style="left: 33.333%"></i><i class="cue-tick" style="left: 66.667%"></i>
          <span class="cue-msg"></span>
          <span class="cue-info" aria-hidden="true">
            <span class="ci ci-target"><small>Tgt</small><b></b></span>
            <span class="ci ci-you"><small></small><b></b><em></em></span>
            <span class="ci ci-next"><small></small><b></b></span>
          </span>
        </div>
        <div class="cue-cap"><span></span></div>
        <div class="cue-target" title="Reference peak brake pressure for this zone">
          <span class="cue-gauge"><i class="cue-gauge-fill"></i><i class="cue-gauge-peak"></i><i class="cue-gauge-target"></i></span>
          <span class="cue-target-text"><small>Target</small><b></b></span>
        </div>
      </div>
      <div class="cue-verdict" aria-live="polite">
        <span class="cue-vzone"></span>
        <span class="cue-chip"></span>
        <span class="cue-vtime"></span>
        <span class="cue-vdist"></span>
        <span class="cue-vpeak"></span>
      </div>
    </div>
    <button class="cue-expand" type="button" aria-label="Expand the brake point window" title="Expand">${EXPAND}</button>
    <div class="ito-size" aria-hidden="true"></div>`;

  const pct = (v) => `${Math.round(v * 100)}%`;
  const signed = (v, digits, unit) => {
    const r = Number(Math.abs(v).toFixed(digits));
    return `${r === 0 ? "" : v < 0 ? "−" : "+"}${r.toFixed(digits)}${unit}`;
  };
  const distText = (m) => (m >= 1000 ? `${(m / 1000).toFixed(1)} km` : `${Math.max(0, Math.round(m))} m`);

  class BrakeCueView {
    constructor(el) {
      el.classList.add("ito-overlay", "ito-cue");
      el.innerHTML = TEMPLATE;
      const $ = (s) => el.querySelector(s);
      Object.assign(this, {
        el,
        handle: $(".cue-head"),
        gear: $(".cue-gear"),
        close: $(".cue-close"),
        collapse: $(".cue-collapse"),
        expand: $(".cue-expand"),
        info: $(".cue-info"),
        ciTarget: $(".ci-target"),
        ciYou: $(".ci-you"),
        ciNext: $(".ci-next"),
        readout: $(".ito-size"),
        body: $(".cue-body"),
        next: $(".cue-next"),
        pipsEl: $(".cue-pips"),
        cap: $(".cue-cap span"),
        msg: $(".cue-msg"),
        target: $(".cue-target-text b"),
        verdict: $(".cue-verdict"),
        vzone: $(".cue-vzone"),
        chip: $(".cue-chip"),
        vtime: $(".cue-vtime"),
        vdist: $(".cue-vdist"),
        vpeak: $(".cue-vpeak"),
      });
      this.pips = [];
    }

    render(s) {
      const b = this.body;
      const mode = s.mode;
      b.dataset.mode = mode;
      b.dataset.beat = s.beat || "";
      b.toggleAttribute("data-joined", s.mode === "countdown" && s.join > 0.001);
      const css = { fill: s.fill || 0, join: s.join || 0, live: s.live || 0, target: s.target || 0, peak: s.peak == null ? -1 : s.peak };
      for (const k in css) b.style.setProperty(`--${k}`, css[k].toFixed(4));
      const f = s.flash;
      if (f) {
        b.dataset.flash = f.grade;
        b.style.setProperty("--flash", GRADES[f.grade].rgb);
        b.style.setProperty("--flash-a", clamp(f.alpha, 0, 1).toFixed(3));
      } else delete b.dataset.flash;

      // BRAKE only when it means it: at the brake point and while the grade shows.
      this.cap.textContent = mode === "countdown" ? String(s.beat) : mode === "brake" || f ? "Brake" : mode === "noref" || mode === "nozones" ? "–" : "";
      this.msg.textContent =
        mode === "idle" ? `NEXT  ${distText(s.dist)}` :
        mode === "noref" ? "NO REFERENCE LAP" :
        mode === "nozones" ? "NO BRAKE ZONES" : "";
      this.target.textContent = s.target != null ? pct(s.target) : "–";
      this.renderCompact(s);
      this.next.textContent = s.zoneNo ? `Z${s.zoneNo}${mode === "countdown" ? ` · ${distText(s.dist)}` : ""}` : "";

      // Timing: the zone being decided, or the last one.
      const v = s.verdict;
      this.verdict.hidden = false;
      this.verdict.classList.toggle("is-empty", !v);
      if (!v) {
        this.vzone.textContent = "";
        this.chip.hidden = true;
        this.vtime.textContent = mode === "noref" ? "Load a Garage 61 lap to get brake points" : "Timing shows after the first zone";
        this.vdist.textContent = this.vpeak.textContent = "";
      } else {
        const g = GRADES[v.grade];
        this.vzone.textContent = v.zoneNo ? `Z${v.zoneNo}` : "–";
        this.chip.hidden = false;
        this.chip.textContent = g.chip;
        this.chip.style.setProperty("--g", g.rgb);
        this.chip.style.color = v.pending ? "" : g.ink || ""; // an outlined, pending chip keeps its own
        this.chip.classList.toggle("is-pending", v.pending);
        this.vtime.textContent = v.dt == null ? "" : signed(v.dt, 2, " s");
        this.vdist.textContent = v.dm == null ? "" : signed(v.dm, 0, " m");
        // Peak vs the zone's target; the difference once you're off the brake.
        const diff = Math.round(v.peak * 100) - Math.round(v.target * 100);
        this.vpeak.innerHTML = v.peak == null ? "" :
          `Peak <b>${pct(v.peak)}</b>${v.current && mode === "braking" ? "" : ` <small>${diff ? signed(diff, 0, "") : "±0"}</small>`}`;
      }
      this.verdict.classList.toggle("is-current", !!(v && v.current));

      // Zone pips.
      const pips = s.pips || [];
      if (this.pips.length !== pips.length) {
        this.pipsEl.innerHTML = pips.map(() => '<i class="cue-pip"></i>').join("");
        this.pips = [...this.pipsEl.children];
      }
      pips.forEach((p, i) => {
        const el = this.pips[i];
        el.style.setProperty("--g", p.grade ? GRADES[p.grade].rgb : "255, 255, 255");
        el.classList.toggle("is-graded", !!p.grade);
        el.classList.toggle("is-stale", p.stale);
        el.classList.toggle("is-current", p.current);
      });
    }

    // Compact mode's readout inside the bar, one number centred in each third:
    //   1. the target for the zone ahead (gold);
    //   2. you (light blue): your pressure now while you're on the brake, then your final
    //      pressure in that zone (its peak) against its target, for 3 s or until the next
    //      countdown starts;
    //   3. the distance to the next brake point, between zones.
    // Without a reference lap, the message spans all three.
    renderCompact(s) {
      const put = (el, label, value, extra) => {
        el.hidden = value == null;
        if (value == null) return;
        el.children[0].textContent = label;
        el.children[1].textContent = value;
        if (el.children[2]) el.children[2].textContent = extra || "";
      };
      const message = s.mode === "noref" ? "No reference lap" : s.mode === "nozones" ? "No brake zones" : null;
      this.info.classList.toggle("is-message", !!message);
      put(this.ciTarget, "Tgt", s.target != null && !message ? pct(s.target) : null);

      const onBrake = (s.live || 0) > 0.02, f = s.final;
      this.ciYou.classList.toggle("is-final", !onBrake && !!f);
      if (onBrake) put(this.ciYou, "Now", pct(s.live));
      else if (f) {
        const diff = Math.round(f.peak * 100) - Math.round(f.target * 100);
        put(this.ciYou, "Final", pct(f.peak), diff ? signed(diff, 0, "") : "±0");
      } else put(this.ciYou, "", null);

      put(this.ciNext, message ? "" : "Next", message || (s.mode === "idle" ? distText(s.dist) : null));
    }
  }

  // ---------------------------------------------------------------- sound

  // A short blip on each count and a long, higher one on BRAKE, like a start-light beep.
  class CueBeeper {
    enable() {
      const AC = root.AudioContext || root.webkitAudioContext;
      if (!this.ctx && AC) this.ctx = new AC();
      if (this.ctx && this.ctx.state === "suspended") this.ctx.resume();
    }
    beep(freq, dur) {
      const c = this.ctx;
      if (!c || c.state !== "running") return;
      const t = c.currentTime, o = c.createOscillator(), g = c.createGain();
      o.type = "sine";
      o.frequency.value = freq;
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(0.2, t + 0.006);
      g.gain.setValueAtTime(0.2, t + dur * 0.7);
      g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
      o.connect(g).connect(c.destination);
      o.start(t);
      o.stop(t + dur + 0.02);
    }
    // Call once per frame with the previous and current state.
    cue(prev, s) {
      if (s.mode === "countdown" && (!prev || prev.mode !== "countdown" || prev.beat !== s.beat)) this.beep(880, 0.08);
      else if (s.mode === "brake" && prev && prev.mode === "countdown") this.beep(1320, 0.3);
    }
  }

  Object.assign(ITO, { BrakeCue, BrakeCueView, CueBeeper, GRADES, gradeOf });
})(typeof window !== "undefined" ? window : globalThis);
