// Wires the overlay together. The SimulatedDriver stands in for iRacing telemetry
// (LapDistPct, Brake, Throttle, SessionTime at 60 Hz) until the SDK is connected.
(function () {
  const ITO = window.ITO;
  const DEFAULTS = {
    bgOpacity: 80,
    refOpacity: 100,
    labels: true,
    labelMode: "both",
    labelMin: 10,
    locked: false,
    axis: "distance",
    historyM: 500,
    aheadM: 500,
    historyS: 8,
    aheadS: 6,
    hz: 60,
    showRef: true,
    // Brake point countdown
    cueOn: true,
    cueLead: 3, // s for the three counts
    cueEarly: 0, // s: show BRAKE this much before the reference brake point
    cueMin: 15, // %: zones with a lower reference peak get no countdown
    cueBeep: false,
    cueTol: 0.08, // s: ± "good" window; "very" early/late past 3×
    cueGraph: true,
    cueFrame: null,
    tab: "display",
    frame: null,
  };
  const HEADER_H = 26;
  const REF_KEY = "ito.reference.v1";
  const settings = new ITO.Settings(DEFAULTS, "ito.settings.v1");

  // ---------- session + reference ----------
  const sample = ITO.parseLapCsv(ITO_SAMPLE_LAP.csv, ITO_SAMPLE_LAP.fileName);
  // Live, these come from iRacing's session info (WeekendInfo.TrackDisplayName / TrackLength).
  const session = { track: sample.meta.track, car: sample.meta.car, trackLength: sample.trackLengthEst };
  let reference = restoreReference();

  function restoreReference() {
    try {
      const raw = localStorage.getItem(REF_KEY);
      if (raw === "none") return null;
      if (raw) {
        const { fileName, csv } = JSON.parse(raw);
        return ITO.parseLapCsv(csv, fileName);
      }
    } catch (e) { /* fall back to the bundled lap */ }
    return sample;
  }
  function storeReference(lap) {
    try {
      localStorage.setItem(REF_KEY, lap ? JSON.stringify({ fileName: `${lap.meta.fileName}.csv`, csv: ITO.serializeLap(lap) }) : "none");
    } catch (e) { /* too big or storage unavailable: keeps working for this session */ }
  }

  // ---------- overlay ----------
  const overlay = document.getElementById("overlay");
  const header = document.getElementById("dragHandle");
  const graph = new ITO.InputGraph(document.getElementById("graph"));
  let panel = null;
  let obstacles = [];
  let dirty = true;

  function defaultFrame() {
    const w = Math.min(680, window.innerWidth - 32), h = 170;
    return { x: Math.round((window.innerWidth - w) / 2), y: Math.max(16, window.innerHeight - h - 64), w, h };
  }

  const frame = ITO.attachFrame(overlay, {
    handle: header,
    readout: document.getElementById("sizeReadout"),
    minW: 260,
    minH: 90,
    onChange: () => panel && panel.position(),
    onCommit: (r) => settings.set("frame", r),
  });
  frame.set(settings.v.frame || defaultFrame());

  // ---------- brake point countdown (its own window) ----------
  const cueEl = document.getElementById("cue");
  const cueView = new ITO.BrakeCueView(cueEl);
  const cue = new ITO.BrakeCue();
  const beeper = new ITO.CueBeeper();
  let cueState = null;

  // Default: centred just above the graph.
  function defaultCueFrame() {
    const o = frame.get(), w = 360, h = 96;
    const y = o.y - h - 12 >= 8 ? o.y - h - 12 : o.y + o.h + 12;
    return { x: Math.round(o.x + (o.w - w) / 2), y, w, h };
  }
  const cueFrame = ITO.attachFrame(cueEl, {
    handle: cueView.handle,
    readout: cueView.readout,
    minW: 230,
    minH: 84,
    onChange: () => panel && panel.position(),
    onCommit: (r) => settings.set("cueFrame", r),
  });
  cueFrame.set(settings.v.cueFrame || defaultCueFrame());
  cueView.close.addEventListener("click", () => settings.set("cueOn", false));

  window.addEventListener("resize", () => { frame.set(frame.get()); cueFrame.set(cueFrame.get()); });

  // Header items the peak labels must steer around (overlay-local px).
  function measureHeader() {
    const o = overlay.getBoundingClientRect();
    const bx = o.left + overlay.clientLeft, by = o.top + overlay.clientTop;
    obstacles = [...header.querySelectorAll(".ito-title, .ito-legend, .ito-gear")]
      .map((el) => el.getBoundingClientRect())
      .filter((r) => r.width > 0)
      .map((r) => ({ x: r.left - bx - 2, y: r.top - by - 2, w: r.width + 4, h: r.height + 4 }));
  }

  new ResizeObserver(() => {
    graph.resize(overlay.clientWidth, overlay.clientHeight);
    measureHeader();
    dirty = true;
  }).observe(overlay);
  document.fonts && document.fonts.ready.then(() => { measureHeader(); dirty = true; });
  // Dragging onto a monitor with different Windows display scaling changes devicePixelRatio.
  (function watchDpr() {
    matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`).addEventListener("change", () => {
      graph.resize(overlay.clientWidth, overlay.clientHeight);
      dirty = true;
      watchDpr();
    }, { once: true });
  })();

  function applyAppearance() {
    const v = settings.v;
    for (const el of [overlay, cueEl]) {
      el.style.setProperty("--bg-alpha", v.bgOpacity / 100);
      el.classList.toggle("is-locked", v.locked);
    }
    cueEl.hidden = !v.cueOn;
    document.getElementById("legendRef").hidden = !reference || !v.showRef;
    document.getElementById("legendRefTime").textContent = reference ? reference.lapTimeText : "–";
    measureHeader();
  }
  settings.on((k) => {
    applyAppearance();
    if (k === "cueBeep" && settings.v.cueBeep) beeper.enable(); // a click, so audio may start
    if (k === "cueTol" || k === "*") renderGradeKey();
    if (k !== "frame" && k !== "cueFrame") dirty = true;
  });

  // Grade key in Settings → Brakes, with the thresholds for the current window.
  function renderGradeKey() {
    const t = settings.v.cueTol, f = (x) => x.toFixed(2);
    const rows = [
      ["veryEarly", `over ${f(3 * t)} s early`],
      ["early", `${f(t)}–${f(3 * t)} s early`],
      ["good", `within ±${f(t)} s`],
      ["late", `${f(t)}–${f(3 * t)} s late`],
      ["veryLate", `over ${f(3 * t)} s late`],
      ["none", "no brake where the reference brakes"],
    ];
    document.getElementById("gradeKey").innerHTML = rows
      .map(([g, text]) => `<span><i class="cue-chip" style="--g: ${ITO.GRADES[g].rgb}">${ITO.GRADES[g].chip}</i>${text}</span>`)
      .join("");
  }
  renderGradeKey();

  panel = ITO.initSettingsPanel({
    panel: document.getElementById("settings"),
    gear: document.getElementById("gear"),
    overlay,
    others: [{ gear: cueView.gear, overlay: cueEl, tab: "brakes" }],
    settings,
    reference: {
      session,
      get: () => reference,
      async load(file) {
        if (!/\.csv$/i.test(file.name)) throw new Error("That isn't a CSV. In Garage 61, open the lap and export it as CSV.");
        reference = ITO.parseLapCsv(await file.text(), file.name);
        storeReference(reference);
        cue.setReference(reference, session.trackLength);
        settings.set("showRef", true);
        applyAppearance();
        panel.sync();
        dirty = true;
      },
      remove() {
        reference = null;
        storeReference(null);
        cue.setReference(null, session.trackLength);
        applyAppearance();
        panel.sync();
        dirty = true;
      },
    },
    onResetLayout: () => {
      settings.set("frame", frame.set(defaultFrame()));
      settings.set("cueFrame", cueFrame.set(defaultCueFrame()));
    },
  });
  applyAppearance();

  // ---------- telemetry (simulated) ----------
  const live = new ITO.LiveTrace();
  const sim = new ITO.SimulatedDriver(sample, session.trackLength);
  // Pre-roll so the first frame already has history behind the car.
  for (let s = sim.step(1 / 60); ; s = sim.step(1 / 60)) {
    live.push(s);
    if (s.D >= 3800) break;
  }
  cue.setReference(reference, session.trackLength);

  const cueCfg = () => {
    const v = settings.v;
    return { lead: v.cueLead, early: v.cueEarly, min: v.cueMin / 100, tol: v.cueTol };
  };
  function updateCue() {
    const prev = cueState;
    cueState = cue.update(live.last, live, cueCfg());
    if (settings.v.cueOn) {
      cueView.render(cueState);
      if (settings.v.cueBeep) beeper.cue(prev, cueState);
    }
  }

  function draw() {
    const v = settings.v;
    const isDist = v.axis === "distance";
    graph.render({
      axis: v.axis,
      behind: isDist ? v.historyM : v.historyS,
      ahead: isDist ? v.aheadM : v.aheadS,
      L: session.trackLength,
      now: live.last,
      live,
      ref: reference,
      showRef: v.showRef,
      refOpacity: v.refOpacity / 100,
      labels: { show: v.labels, mode: v.labelMode, min: v.labelMin / 100 },
      cue: v.cueGraph && cueState && cueState.cueZones ? cueState : null,
      headerH: HEADER_H,
      obstacles,
    });
  }

  // ---------- loop ----------
  let playing = true, speed = 1, acc = 0, last = performance.now();
  function tick(t) {
    const dt = Math.min(0.5, (t - last) / 1000); // don't fast-forward after the tab was hidden
    last = t;
    if (playing) {
      acc += dt * speed;
      const step = 1 / settings.v.hz;
      let n = 0;
      while (acc >= step && n < 240) { live.push(sim.step(step)); acc -= step; n++; }
      if (n) {
        const s = live.last;
        updateCue();
        live.prune(s.t - 25, s.D - 1600); // widest window any setting can show
        updateHarness();
        dirty = true;
      }
    }
    if (dirty) {
      dirty = false;
      if (!playing) updateCue(); // settings changed while paused
      draw();
    }
    requestAnimationFrame(tick);
  }

  // ---------- prototype harness ----------
  const playBtn = document.getElementById("playBtn");
  const ICON_PAUSE = '<svg viewBox="0 0 12 12" aria-hidden="true"><rect x="2" y="1.5" width="3" height="9" rx="1"/><rect x="7" y="1.5" width="3" height="9" rx="1"/></svg>';
  const ICON_PLAY = '<svg viewBox="0 0 12 12" aria-hidden="true"><path d="M3 1.8v8.4a.6.6 0 0 0 .9.5l7-4.2a.6.6 0 0 0 0-1l-7-4.2a.6.6 0 0 0-.9.5z"/></svg>';
  function setPlaying(p) {
    playing = p;
    playBtn.innerHTML = p ? ICON_PAUSE : ICON_PLAY;
    playBtn.setAttribute("aria-label", p ? "Pause" : "Play");
    last = performance.now();
  }
  playBtn.addEventListener("click", () => setPlaying(!playing));
  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, button, [role=button], [role=tab]")) return;
    if (e.code === "Space") {
      e.preventDefault();
      setPlaying(!playing);
    } else if (e.code === "KeyN") skipToNextZone();
  });

  // Fast-forward to a second before the next countdown starts.
  function skipToNextZone() {
    const target = cue.ref && cue.nextArmAt(live.last, cueCfg());
    if (target == null) return;
    for (let i = 0; i < 60 * 200; i++) {
      const s = sim.step(1 / 60);
      live.push(s);
      if (i % 600 === 599) updateCue(); // grade zones on the way past
      if (cue.clock(s.lap, s.pct) >= target) break;
    }
    updateCue();
    live.prune(live.last.t - 25, live.last.D - 1600);
    updateHarness();
    dirty = true;
  }
  document.getElementById("nextZoneBtn").addEventListener("click", skipToNextZone);

  // Step the sim by `sec` and redraw now (from the console, e.g. to catch a state).
  function advance(sec) {
    for (let i = 0; i < Math.round(sec * 60); i++) {
      live.push(sim.step(1 / 60));
      updateCue();
    }
    live.prune(live.last.t - 25, live.last.D - 1600);
    updateHarness();
    draw();
    return cueState;
  }
  const speedSeg = document.getElementById("speedSeg");
  speedSeg.addEventListener("click", (e) => {
    const b = e.target.closest("button[data-speed]");
    if (!b) return;
    speed = Number(b.dataset.speed);
    for (const x of speedSeg.children) x.setAttribute("aria-pressed", String(x === b));
  });
  document.getElementById("hzTrack").textContent = session.track;
  function updateHarness() {
    const tl = sim.t - sim.lapStart;
    document.getElementById("hzLap").textContent = sim.lapIndex + 1;
    document.getElementById("hzClock").textContent = `${Math.floor(tl / 60)}:${(tl % 60).toFixed(1).padStart(4, "0")}`;
  }

  setPlaying(true);
  updateCue();
  updateHarness();
  requestAnimationFrame(tick);

  // Handy from the dev console.
  window.ITO_APP = {
    settings, live, sim, graph, panel, setPlaying, cue, cueView, skipToNextZone, advance,
    get reference() { return reference; },
    get cueState() { return cueState; },
  };
})();
