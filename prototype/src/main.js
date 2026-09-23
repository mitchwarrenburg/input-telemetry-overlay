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
  window.addEventListener("resize", () => frame.set(frame.get()));

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
    overlay.style.setProperty("--bg-alpha", v.bgOpacity / 100);
    overlay.classList.toggle("is-locked", v.locked);
    document.getElementById("legendRef").hidden = !reference || !v.showRef;
    document.getElementById("legendRefTime").textContent = reference ? reference.lapTimeText : "–";
    measureHeader();
  }
  settings.on((k) => {
    applyAppearance();
    if (k !== "frame") dirty = true;
  });

  panel = ITO.initSettingsPanel({
    panel: document.getElementById("settings"),
    gear: document.getElementById("gear"),
    overlay,
    settings,
    reference: {
      session,
      get: () => reference,
      async load(file) {
        if (!/\.csv$/i.test(file.name)) throw new Error("That isn't a CSV. In Garage 61, open the lap and export it as CSV.");
        reference = ITO.parseLapCsv(await file.text(), file.name);
        storeReference(reference);
        settings.set("showRef", true);
        applyAppearance();
        panel.sync();
        dirty = true;
      },
      remove() {
        reference = null;
        storeReference(null);
        applyAppearance();
        panel.sync();
        dirty = true;
      },
    },
    onResetLayout: () => settings.set("frame", frame.set(defaultFrame())),
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
        live.prune(s.t - 25, s.D - 1600); // widest window any setting can show
        updateHarness();
        dirty = true;
      }
    }
    if (dirty) { dirty = false; draw(); }
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
    if (e.code === "Space" && !e.target.closest("input, button, [role=button], [role=tab]")) {
      e.preventDefault();
      setPlaying(!playing);
    }
  });
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
  updateHarness();
  requestAnimationFrame(tick);

  // Handy from the dev console.
  window.ITO_APP = { settings, live, sim, graph, panel, setPlaying, get reference() { return reference; } };
})();
