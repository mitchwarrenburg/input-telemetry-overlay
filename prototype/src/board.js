// Design board (brake-cue.html): the brake point window in fixed states, drawn by the
// same BrakeCueView the live prototype uses.
(function () {
  const ITO = window.ITO;
  const $ = (s) => document.querySelector(s);

  // ---------- fixed states ----------
  // Nine zones; this lap has Z1–Z3 graded, Z4 is next, the rest show last lap.
  const LAST_LAP = ["good", "late", "perfect", "early", "good", "veryLate", "perfect", "early", "good"];
  function pips(thisLap, current = 3) {
    return LAST_LAP.map((g, i) => ({
      grade: i < thisLap.length ? thisLap[i] : g,
      stale: i >= thisLap.length,
      current: i === current,
    }));
  }
  const verdict = (grade, dt, dm, peak, target, zoneNo, extra) =>
    ({ grade, dt, dm, peak, target, zoneNo, pending: false, current: false, ...extra });
  const Z3 = verdict("good", -0.05, -3, 0.61, 0.62, 3);
  const base = {
    mode: "countdown", beat: 2, fill: 0.5, join: 0,
    zoneNo: 4, zoneCount: 9, dist: 124, target: 0.72, live: 0, peak: null,
    verdict: Z3, pips: pips(["good", "late", "good"]),
  };
  const S = (o) => ({ ...base, ...o });

  // opts: { compact, bg, fg, hover }: compact layout, background and contents opacity
  // (0–1), and the expand button shown as if hovered.
  function widget(state, w = 360, h = 96, opts = {}) {
    const el = document.createElement("section");
    const view = new ITO.BrakeCueView(el);
    el.classList.add("is-locked");
    el.classList.toggle("is-compact", !!opts.compact);
    el.classList.toggle("is-hover-demo", !!opts.hover);
    el.classList.toggle("is-pulse-demo", !!opts.pulse);
    if (opts.bg != null) el.style.setProperty("--bg-alpha", opts.bg);
    if (opts.fg != null) el.style.setProperty("--fg-alpha", opts.fg);
    el.style.width = `${w}px`;
    el.style.height = `${h}px`;
    el.setAttribute("aria-label", "Brake point countdown");
    view.render(state);
    return el;
  }

  function card(host, { tag, title, note, state, w, h, opts }) {
    const c = document.createElement("div");
    c.className = "spec";
    const t = document.createElement("div");
    t.className = "cap-title";
    t.innerHTML = tag ? `<span>${tag}</span>` : "";
    t.append(title);
    const bd = document.createElement("div");
    bd.className = "backdrop";
    bd.append(widget(state, w, h, opts));
    c.append(t, bd);
    if (note) {
      const p = document.createElement("p");
      p.className = "note";
      p.innerHTML = note;
      c.append(p);
    }
    host.append(c);
  }

  // ---------- anatomy ----------
  const anatomy = $("#anatomy");
  const big = widget(S({ fill: 0.52, dist: 118 }), 520, 132);
  anatomy.append(big);
  // Numbered pins with leader lines, placed from the rendered layout.
  function placePins() {
    anatomy.querySelectorAll(".pin, .leader").forEach((n) => n.remove());
    const box = anatomy.getBoundingClientRect();
    const pins = [
      [".cue-next", "up"], [".cue-pips", "up"], [".cue-track", "left"],
      [".cue-cap", "down"], [".cue-target", "right"], [".cue-chip", "down"],
    ];
    pins.forEach(([sel, dir], i) => {
      const r = big.querySelector(sel).getBoundingClientRect();
      const w = big.getBoundingClientRect();
      const cx = r.left + r.width / 2 - box.left, cy = r.top + r.height / 2 - box.top;
      let px, py, line;
      if (dir === "up") { px = cx; py = w.top - box.top - 16; line = { x: cx, y: py, w: 1, h: r.top - box.top - py }; }
      if (dir === "down") { px = cx; py = w.bottom - box.top + 16; line = { x: cx, y: r.bottom - box.top, w: 1, h: py - (r.bottom - box.top) }; }
      if (dir === "left") { px = w.left - box.left - 18; py = cy; line = { x: px, y: cy, w: r.left - box.left + 14 - px, h: 1 }; }
      if (dir === "right") { px = w.right - box.left + 18; py = cy; line = { x: r.right - box.left, y: cy, w: px - (r.right - box.left), h: 1 }; }
      const ln = document.createElement("i");
      ln.className = "leader";
      Object.assign(ln.style, { position: "absolute", zIndex: 5, left: `${line.x}px`, top: `${line.y}px`, width: `${line.w}px`, height: `${line.h}px`, background: "rgba(46, 230, 160, 0.8)" });
      const pin = document.createElement("span");
      pin.className = "dot-n pin";
      pin.textContent = i + 1;
      Object.assign(pin.style, { left: `${px}px`, top: `${py}px` });
      anatomy.append(ln, pin);
    });
  }
  placePins();
  document.fonts && document.fonts.ready.then(placePins);
  window.addEventListener("resize", placePins);

  // ---------- the sequence ----------
  const seq = $("#seq");
  [
    { tag: "Approach", title: "Next zone", state: S({ mode: "idle", beat: 0, fill: 0, dist: 540 }),
      note: "Off the brake and not yet in range: distance to the next brake point, its target, and the last zone's result." },
    { tag: "−3 s", title: "3", state: S({ beat: 3, fill: 0.08, dist: 186 }), note: "The count starts 3 s out (a setting), filling in red." },
    { tag: "−2 s", title: "2", state: S({ beat: 2, fill: 0.4, dist: 124 }), note: "The fill moves continuously, the red darkening as it goes." },
    { tag: "−1 s", title: "1", state: S({ beat: 1, fill: 0.82, dist: 40 }), note: "Darkest just before the brake point." },
    { tag: "0 s", title: "BRAKE", state: S({ mode: "brake", beat: 0, fill: 1, dist: 0 }), note: "At the reference brake point: the whole bar and the cap solid red, with a slight glow." },
    { tag: "+0.1 s", title: "Graded", state: S({ mode: "braking", beat: 0, fill: 1, live: 0.5, peak: 0.5, flash: { grade: "good", alpha: 1 }, verdict: verdict("good", 0.05, 3, 0.5, 0.72, 4, { current: true }), pips: pips(["good", "late", "good", "good"]) }),
      note: "The moment you brake, the bar takes your grade's colour for 0.8 s, then fades out over 0.3 s." },
    { tag: "+1.1 s", title: "Emptied", state: S({ mode: "braking", beat: 0, fill: 1, live: 0.66, peak: 0.7, verdict: verdict("good", 0.05, 3, 0.7, 0.72, 4, { current: true }), pips: pips(["good", "late", "good", "good"]) }),
      note: "Empty again while you're still braking; the gauge fills toward the target line." },
    { tag: "Released", title: "Result", state: S({ mode: "idle", beat: 0, fill: 0, zoneNo: 5, dist: 412, target: 0.48, verdict: verdict("good", 0.05, 3, 0.7, 0.72, 4), pips: pips(["good", "late", "good", "good"], 4) }),
      note: "The result holds until the next zone is decided; the pip for Z4 turns green." },
  ].forEach((s) => card(seq, { ...s, w: 236, h: 96 }));

  // ---------- grade scale ----------
  const tol = 0.08, perfect = 0.03;
  // Tightest first: Perfect sits inside Good, so an early-to-late order would mislead.
  $("#scale").innerHTML = [
    ["perfect", `within ±${perfect.toFixed(2)} s`, "On the reference brake point."],
    ["good", `within ±${tol.toFixed(2)} s`, "Close to the reference brake point."],
    ["early", `${tol.toFixed(2)}–${(3 * tol).toFixed(2)} s early`, "Room to brake later."],
    ["late", `${tol.toFixed(2)}–${(3 * tol).toFixed(2)} s late`, "Later than the reference; check you still make the apex."],
    ["veryEarly", `over ${(3 * tol).toFixed(2)} s early`, "Braking well before the reference. Time left on the table."],
    ["veryLate", `over ${(3 * tol).toFixed(2)} s late`, "Likely overshooting."],
    ["none", "no brake-on", "Lifted or stayed flat where the reference brakes."],
  ].map(([g, range, text]) => {
    const G = ITO.GRADES[g];
    return `<div><i class="cue-chip" style="--g: ${G.rgb}; color: ${G.ink || ""}; font-style: normal">${G.chip}</i><strong>${G.label}</strong><small>${range}</small><small>${text}</small></div>`;
  }).join("");

  // ---------- other cases ----------
  const cases = $("#cases");
  [
    { tag: "Perfect", title: "Within ±0.03 s",
      state: S({ mode: "braking", beat: 0, fill: 1, live: 0.58, peak: 0.58, flash: { grade: "perfect", alpha: 1 }, verdict: verdict("perfect", 0.01, 1, 0.58, 0.72, 4, { current: true }), pips: pips(["good", "late", "good", "perfect"]) }),
      note: "The whole bar and the cap light purple with a glow, then empty like any other grade." },
    { tag: "Early", title: "Braked before the count ended",
      state: S({ mode: "braking", beat: 0, fill: 0.88, live: 0.52, peak: 0.52, flash: { grade: "veryEarly", alpha: 1 }, verdict: verdict("veryEarly", -0.27, -19, 0.52, 0.72, 4, { current: true }), pips: pips(["good", "late", "good", "veryEarly"]) }),
      note: "The bar stops where you braked and shows the early colour; the gap to the cap is how early." },
    { tag: "Try-out", title: "Red pulse on BRAKE", state: S({ mode: "brake", beat: 0, fill: 1, dist: 0 }), opts: { pulse: true },
      note: "As the brake point arrives, both windows' backgrounds (this one and the graph) flash red for half a second, under their contents. Shown at its peak; a switch in Countdown turns it off." },
    { tag: "Late", title: "Past the brake point, not braking yet",
      state: S({ mode: "brake", beat: 0, fill: 1, dist: -8, verdict: verdict("late", 0.13, 9, null, 0.72, 4, { pending: true, current: true }) }),
      note: "The bar stays solid red and the outlined grade counts up until you brake." },
    { tag: "Late", title: "Braked late",
      state: S({ mode: "braking", beat: 0, fill: 1, live: 0.8, peak: 0.84, flash: { grade: "veryLate", alpha: 1 }, verdict: verdict("veryLate", 0.29, 21, 0.84, 0.72, 4, { current: true }), pips: pips(["good", "late", "good", "veryLate"]) }),
      note: "Graded when the brake goes on; the gauge's pink mark is your highest pressure so far." },
    { tag: "Short straight", title: "Count joins at 2",
      state: S({ beat: 2, fill: 0.55, join: 0.4, zoneNo: 2, dist: 58, target: 0.46, verdict: verdict("late", 0.12, 7, 0.7, 0.68, 1), pips: pips(["late"], 1) }),
      note: "The previous zone ended less than 3 s before this brake point. The skipped part is hatched." },
    { tag: "Missed", title: "Drove through a zone",
      state: S({ mode: "idle", beat: 0, fill: 0, zoneNo: 7, dist: 610, target: 0.33, verdict: verdict("none", null, null, null, 0.3, 6), pips: pips(["good", "late", "good", "good", "early", "none"], 6) }),
      note: "The reference braked; you lifted or stayed flat." },
    { tag: "Empty", title: "No reference lap", state: { mode: "noref" },
      note: "Nothing to count to until a lap is loaded." },
  ].forEach((c) => card(cases, c));

  // ---------- sizes ----------
  const sizes = $("#sizes");
  card(sizes, { tag: "360 × 96", title: "Default", state: S({ fill: 0.5 }), note: "Placed above the graph at first; move it anywhere." });
  card(sizes, { tag: "250 × 96", title: "Narrow", state: S({ fill: 0.5 }), w: 250, h: 96,
    note: "Below 330 px the zone strip goes; below 290 px the metres; below 250 px the peak." });
  card(sizes, { tag: "480 × 128", title: "Large", state: S({ beat: 1, fill: 0.8, dist: 46 }), w: 480, h: 128,
    note: "The count and cap text scale with the window's height." });

  // ---------- compact ----------
  const compact = $("#compact");
  const C = (o) => ({ w: 360, h: 40, ...o, opts: { compact: true, ...(o.opts || {}) } });
  [
    { tag: "Approach", title: "Next zone", state: S({ mode: "idle", beat: 0, fill: 0, dist: 540 }),
      note: "One number centred in each third. Left: the next zone's target (gold). Right: the distance to its brake point. The middle is yours, and empty between zones." },
    { tag: "−2 s", title: "Counting", state: S({ beat: 2, fill: 0.45, dist: 118 }),
      note: "The count fills the bar as in the full window; the target stays." },
    { tag: "0 s", title: "BRAKE", state: S({ mode: "brake", beat: 0, fill: 1, dist: 0 }), note: "Solid red with the glow; just the target." },
    { tag: "Braking", title: "Graded", state: S({ mode: "braking", beat: 0, fill: 1, live: 0.5, peak: 0.5, flash: { grade: "good", alpha: 1 }, verdict: verdict("good", 0.05, 3, 0.5, 0.72, 4, { current: true }) }),
      note: "On the brake, the middle shows your pressure now, in light blue." },
    { tag: "Braking", title: "Still on the brake", state: S({ mode: "braking", beat: 0, fill: 1, live: 0.66, peak: 0.7, verdict: verdict("good", 0.05, 3, 0.7, 0.72, 4, { current: true }) }),
      note: "The bar has emptied; your pressure keeps updating." },
    { tag: "Released", title: "Final", state: S({ mode: "idle", beat: 0, fill: 0, zoneNo: 5, dist: 412, target: 0.48, final: { peak: 0.7, target: 0.72, age: 0.5 }, verdict: verdict("good", 0.05, 3, 0.7, 0.72, 4) }),
      note: "Off the brake, the middle becomes your final pressure for that zone (its peak) against its target, for 3 s or until the next countdown starts. The gold target has moved on to the next zone." },
    { tag: "+3 s", title: "Final gone", state: S({ mode: "idle", beat: 0, fill: 0, zoneNo: 5, dist: 318, target: 0.48, verdict: verdict("good", 0.05, 3, 0.7, 0.72, 4) }),
      note: "Back to the target and the distance." },
    { tag: "Perfect", title: "Purple", state: S({ mode: "braking", beat: 0, fill: 1, live: 0.58, peak: 0.58, flash: { grade: "perfect", alpha: 1 }, verdict: verdict("perfect", 0.01, 1, 0.58, 0.72, 4, { current: true }) }),
      note: "The whole bar and the cap light purple, as in the full window." },
    { tag: "Hover", title: "Expand", state: S({ mode: "idle", beat: 0, fill: 0, dist: 540 }), opts: { hover: true },
      note: "Hovering shows the expand button. The whole bar moves the window; the edges resize it." },
    { tag: "230 × 34", title: "Narrow", state: S({ mode: "idle", beat: 0, fill: 0, dist: 540, final: { peak: 0.61, target: 0.62, age: 1 } }), w: 230, h: 34,
      note: "Below 250 px the small labels go and only the numbers stay." },
  ].forEach((c) => card(compact, C(c)));

  // ---------- transparency ----------
  const alpha = $("#alpha");
  const busy = S({ beat: 1, fill: 0.8, dist: 42 });
  [
    { tag: "80% · 100%", title: "Default", state: busy, opts: { bg: 0.8, fg: 1 } },
    { tag: "30% · 100%", title: "Light background", state: busy, opts: { bg: 0.3, fg: 1 } },
    { tag: "0% · 70%", title: "No background, softer contents", state: busy, opts: { bg: 0, fg: 0.7 } },
    { tag: "0% · 85%", title: "Compact, no background", state: S({ mode: "braking", beat: 0, fill: 1, live: 0.66, peak: 0.7, verdict: verdict("good", 0.05, 3, 0.7, 0.72, 4, { current: true }) }),
      w: 360, h: 40, opts: { compact: true, bg: 0, fg: 0.85 } },
  ].forEach((c) => card(alpha, c));

  // ---------- graph ----------
  const sample = ITO.parseLapCsv(ITO_SAMPLE_LAP.csv, ITO_SAMPLE_LAP.fileName);
  const L = sample.trackLengthEst;
  const live = new ITO.LiveTrace();
  const sim = new ITO.SimulatedDriver(sample, L, 4);
  const cue = new ITO.BrakeCue();
  cue.setReference(sample, L);
  const cfg = { lead: 3, early: 0, min: 0.15, tol, perfect };
  let st = null;
  for (let s = sim.step(1 / 60); ; s = sim.step(1 / 60)) {
    live.push(s);
    st = cue.update(s, live, cfg);
    if (s.D >= 4300) break;
  }
  const host = $("#graphOverlay");
  host.insertAdjacentHTML("beforeend",
    '<header class="ito-header"><span class="ito-title">Throttle / Brake<span class="unit">%</span></span></header>');
  const graph = new ITO.InputGraph($("#graph"));
  function draw() {
    graph.resize(host.clientWidth, host.clientHeight);
    graph.render({
      axis: "distance", behind: 560, ahead: 600, L, now: live.last, live, ref: sample, showRef: true, refOpacity: 1,
      labels: { show: true, mode: "both", min: 0.1 }, headerH: 26, obstacles: [], cue: st,
    });
  }
  draw();
  window.addEventListener("resize", draw);
  document.fonts && document.fonts.ready.then(draw);

  // ---------- peak labels ----------
  // The same lap, seed and positions every time, so the shots can be compared.
  function shot(host, lapD, w, h, note) {
    const live = new ITO.LiveTrace(), sim = new ITO.SimulatedDriver(sample, L, 7), cue = new ITO.BrakeCue();
    cue.setReference(sample, L);
    let st;
    for (let s = sim.step(1 / 60); ; s = sim.step(1 / 60)) {
      live.push(s);
      st = cue.update(s, live, cfg);
      if (s.D >= L + lapD) break;
    }
    const el = document.createElement("div");
    el.className = "shot";
    el.innerHTML = `<div class="cap-title"><span>${w} × ${h}</span>${Math.round(lapD)} m</div>
      <div class="frame"><section class="ito-overlay is-locked" style="width: ${w}px; height: ${h}px"><canvas class="ito-canvas"></canvas>
      <header class="ito-header"><span class="ito-title"><span class="full">Throttle / Brake</span><span class="short">Thr / Brk</span><span class="unit">%</span></span></header></section></div>
      <p class="note">${note}</p>`;
    host.append(el);
    const ov = el.querySelector("section");
    const g = new ITO.InputGraph(el.querySelector("canvas"));
    const render = () => {
      g.resize(ov.clientWidth, ov.clientHeight);
      const t = ov.querySelector(".ito-title").getBoundingClientRect(), o = ov.getBoundingClientRect();
      g.render({
        axis: "distance", behind: 500, ahead: 500, L, now: live.last, live, ref: sample, showRef: true, refOpacity: 1,
        labels: { show: true, mode: "both", min: 0.1 }, headerH: 26, cue: st,
        obstacles: [{ x: t.left - o.left - 2, y: t.top - o.top - 2, w: t.width + 4, h: t.height + 4 }],
      });
    };
    render();
    document.fonts && document.fonts.ready.then(render);
  }
  shot($("#shotsWide"), 1060, 680, 170,
    "Two zones behind the car. The reference peaks' numbers sit in the row along the top, each on a dotted line through its peak; yours sit on the apex of your brake line.");
  shot($("#shotsWide"), 3960, 680, 170,
    "A tight run of zones: the dotted lines show where the reference peaked, so you can see yours land before or after it.");
  shot($("#shotsWide"), 4980, 680, 170,
    "The next zone's reference peak in the look-ahead: its number waits at the top of its line.");
  shot($("#shots"), 3960, 300, 190, "Narrow: the same labels, in the same places.");
  shot($("#shots"), 1060, 272, 90, "The smallest graph: the reference row fills the top, so your peak labels hang below their apexes.");
  shot($("#shots"), 2140, 272, 90, "A high peak of yours near a reference label: your label drops below the apex instead.");
})();
