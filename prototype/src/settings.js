// Settings store (persisted) and the gear-menu popover that edits it.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  class Settings {
    constructor(defaults, key) {
      this.defaults = defaults;
      this.key = key;
      this.listeners = new Set();
      let saved = {};
      try { saved = JSON.parse(localStorage.getItem(key)) || {}; } catch (e) { /* storage unavailable */ }
      this.v = { ...defaults, ...saved };
    }
    set(k, val) {
      if (this.v[k] === val) return;
      this.v[k] = val;
      this.save();
      this.listeners.forEach((fn) => fn(k, val));
    }
    reset(keep = []) {
      const kept = Object.fromEntries(keep.map((k) => [k, this.v[k]]));
      this.v = { ...this.defaults, ...kept };
      this.save();
      this.listeners.forEach((fn) => fn("*"));
    }
    on(fn) { this.listeners.add(fn); }
    save() {
      clearTimeout(this.timer);
      this.timer = setTimeout(() => {
        try { localStorage.setItem(this.key, JSON.stringify(this.v)); } catch (e) { /* storage unavailable */ }
      }, 150);
    }
  }

  const ICON_OK = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>';
  const ICON_WARN = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4 2.8 19.5h18.4L12 4zM12 10v4.2M12 17v.2"/></svg>';

  // reference: { session, get(), load(file) → Promise, remove() }
  // others: more windows with their own gear, [{ gear, overlay, tab }]; the panel opens
  // beside whichever window's gear opened it, on that window's tab.
  function initSettingsPanel({ panel, gear, overlay, others = [], settings, reference, onResetLayout }) {
    const $ = (s) => panel.querySelector(s);
    const $$ = (s) => [...panel.querySelectorAll(s)];
    const isOpen = () => !panel.hidden;
    const owners = [{ gear, overlay }, ...others];
    let owner = owners[0];

    // ---------- open, close, placement ----------
    // Sits outside the overlay (below, else above, else beside) so changes stay visible.
    function position() {
      if (!isOpen()) return;
      const o = (owner.overlay.hidden ? owners[0] : owner).overlay.getBoundingClientRect();
      const pw = panel.offsetWidth, ph = panel.offsetHeight;
      const vw = root.innerWidth, vh = root.innerHeight, gap = 10, pad = 8;
      let left = Math.max(pad, Math.min(vw - pw - pad, o.right - pw));
      let top, from;
      if (o.bottom + gap + ph <= vh - pad) { top = o.bottom + gap; from = "-4px"; }
      else if (o.top - gap - ph >= pad) { top = o.top - gap - ph; from = "4px"; }
      else {
        top = Math.max(pad, Math.min(vh - ph - pad, o.top));
        left = o.right + gap + pw <= vw - pad ? o.right + gap : Math.max(pad, o.left - gap - pw);
        from = "0px";
      }
      panel.style.left = `${Math.round(left)}px`;
      panel.style.top = `${Math.round(top)}px`;
      panel.style.setProperty("--pop-from", from);
    }

    function open(tab, from = owners[0]) {
      if (owner !== from) owner.gear.setAttribute("aria-expanded", "false");
      owner = from;
      panel.hidden = false;
      owner.gear.setAttribute("aria-expanded", "true");
      selectTab(tab || settings.v.tab || "display");
      sync();
      position();
    }
    function close() {
      if (!isOpen()) return;
      panel.hidden = true;
      owner.gear.setAttribute("aria-expanded", "false");
    }

    for (const o of owners) {
      o.gear.addEventListener("click", () => (isOpen() && owner === o ? close() : open(o.tab, o)));
    }
    $("[data-close]").addEventListener("click", () => { close(); owner.gear.focus(); });
    document.addEventListener("pointerdown", (e) => {
      if (isOpen() && !panel.contains(e.target) && !owners.some((o) => o.overlay.contains(e.target))) close();
    });
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && isOpen()) { close(); owner.gear.focus(); }
    });
    root.addEventListener("resize", position);

    // ---------- tabs ----------
    function selectTab(name) {
      if (!$$("[role=tab]").some((t) => t.dataset.tab === name)) name = "display"; // e.g. a renamed tab
      for (const t of $$("[role=tab]")) {
        const on = t.dataset.tab === name;
        t.setAttribute("aria-selected", String(on));
        t.tabIndex = on ? 0 : -1;
      }
      for (const p of $$("[role=tabpanel]")) p.hidden = p.id !== `panel-${name}`;
      settings.set("tab", name);
      position();
    }
    for (const t of $$("[role=tab]")) t.addEventListener("click", () => selectTab(t.dataset.tab));
    $(".set-tabs").addEventListener("keydown", (e) => {
      if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
      const tabs = $$("[role=tab]");
      const i = tabs.indexOf(document.activeElement);
      if (i < 0) return;
      const next = tabs[(i + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
      next.focus();
      selectTab(next.dataset.tab);
    });

    // ---------- controls (bound by data-setting) ----------
    for (const inp of $$("input[type=range][data-setting]")) {
      inp.addEventListener("input", () => settings.set(inp.dataset.setting, parseFloat(inp.value)));
    }
    for (const seg of $$(".seg[data-setting]")) {
      seg.addEventListener("click", (e) => {
        const b = e.target.closest("button[data-value]");
        if (!b) return;
        const k = seg.dataset.setting;
        settings.set(k, typeof settings.defaults[k] === "number" ? Number(b.dataset.value) : b.dataset.value);
      });
    }
    for (const inp of $$("input[type=checkbox][data-setting]")) {
      inp.addEventListener("change", () => settings.set(inp.dataset.setting, inp.checked));
    }
    $("#resetAll").addEventListener("click", () => settings.reset(["frame", "cueFrame", "cueCompactFrame", "tab"]));
    $("#resetLayout").addEventListener("click", () => onResetLayout && onResetLayout());

    function format(k, v, inp) {
      if ((k.startsWith("ahead") || k === "cueEarly") && v === 0) return "Off";
      const d = inp.dataset.decimals;
      return `${inp.dataset.prefix || ""}${d ? v.toFixed(Number(d)) : Number.isInteger(v) ? v : v.toFixed(1)}${inp.dataset.unit || ""}`;
    }

    function sync() {
      const v = settings.v;
      for (const inp of $$("input[type=range][data-setting]")) {
        const k = inp.dataset.setting;
        inp.value = v[k];
        inp.style.setProperty("--fill", `${((v[k] - inp.min) / (inp.max - inp.min)) * 100}%`);
        const out = panel.querySelector(`output[data-for="${k}"]`);
        if (out) out.textContent = format(k, v[k], inp);
      }
      for (const seg of $$(".seg[data-setting]")) {
        for (const b of seg.querySelectorAll("button")) {
          b.setAttribute("aria-pressed", String(b.dataset.value === String(v[seg.dataset.setting])));
        }
      }
      for (const inp of $$("input[type=checkbox][data-setting]")) inp.checked = !!v[inp.dataset.setting];
      for (const r of $$("[data-axis]")) r.hidden = r.dataset.axis !== v.axis;
      for (const r of $$("[data-needs]")) r.classList.toggle("is-disabled", !v[r.dataset.needs]);
      renderReference();
    }
    settings.on((k) => { if (k !== "frame") sync(); });

    // ---------- reference lap ----------
    function renderReference() {
      const lap = reference.get();
      $("#refCard").hidden = !lap;
      $("#dropzone").classList.toggle("is-compact", !!lap);
      $("#dzTitle").textContent = lap ? "Drop another lap here to replace it" : "Drop a Garage 61 lap CSV";
      if (!lap) return;
      const m = lap.meta;
      const s = reference.session;
      $("#refSrc").textContent = m.source === "Garage 61" ? "G61" : "CSV";
      $("#refDriver").textContent = m.driver || m.fileName;
      $("#refTime").textContent = lap.lapTimeText;
      $("#refSub").textContent = [m.car, m.track].filter(Boolean).join(" · ") || "Car and track not in file name";
      $("#refStats").textContent = `${lap.n.toLocaleString()} samples · ${Math.round(lap.hz)} Hz` +
        (lap.trackLengthEst ? ` · ${(lap.trackLengthEst / 1000).toFixed(2)} km` : "");
      let ok = true, text = "Matches session";
      if (!m.track) { ok = false; text = "Track unknown"; }
      else if (s.track && m.track !== s.track) { ok = false; text = "Different track"; }
      else if (m.car && s.car && m.car !== s.car) { ok = false; text = "Different car"; }
      const st = $("#refStatus");
      st.className = `ref-status ${ok ? "ok" : "warn"}`;
      st.innerHTML = `${ok ? ICON_OK : ICON_WARN}<span></span>`;
      st.lastChild.textContent = text;
    }

    const fileInput = $("#fileInput");
    const errorBox = $("#refError");
    function showError(msg) {
      errorBox.textContent = msg || "";
      errorBox.hidden = !msg;
      position();
    }
    async function take(file) {
      if (!file) return;
      showError("");
      try { await reference.load(file); } catch (e) { showError(e.message || String(e)); }
      if (!isOpen()) open("reference"); else selectTab("reference");
    }

    const hasFiles = (e) => [...((e.dataTransfer && e.dataTransfer.types) || [])].includes("Files");
    const dropzone = $("#dropzone");
    for (const b of $$("[data-browse]")) b.addEventListener("click", () => fileInput.click());
    dropzone.addEventListener("click", () => fileInput.click());
    dropzone.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); fileInput.click(); }
    });
    fileInput.addEventListener("change", () => { take(fileInput.files[0]); fileInput.value = ""; });
    for (const [el, cls] of [[dropzone, "is-over"], [overlay, "is-dragover"]]) {
      let depth = 0;
      el.addEventListener("dragenter", (e) => { if (hasFiles(e)) { e.preventDefault(); depth++; el.classList.add(cls); } });
      el.addEventListener("dragover", (e) => { if (hasFiles(e)) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; } });
      el.addEventListener("dragleave", () => { if (--depth <= 0) { depth = 0; el.classList.remove(cls); } });
      el.addEventListener("drop", (e) => {
        if (!hasFiles(e)) return;
        e.preventDefault();
        e.stopPropagation();
        depth = 0;
        el.classList.remove(cls);
        take(e.dataTransfer.files[0]);
      });
    }
    // A file dropped anywhere else shouldn't navigate the page away.
    root.addEventListener("dragover", (e) => e.preventDefault());
    root.addEventListener("drop", (e) => e.preventDefault());
    $("#refRemove").addEventListener("click", () => { reference.remove(); showError(""); });

    sync();
    return { open, close, position, sync, take };
  }

  Object.assign(ITO, { Settings, initSettingsPanel });
})(typeof window !== "undefined" ? window : globalThis);
