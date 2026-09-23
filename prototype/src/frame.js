// Move (drag the header) and resize (8 anchors) for a position:fixed element.
// In the Electron build these deltas become BrowserWindow.setBounds() calls.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});
  const DIRS = ["n", "ne", "e", "se", "s", "sw", "w", "nw"];
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  ITO.attachFrame = function (el, { handle, readout, minW = 260, minH = 90, onChange, onCommit }) {
    const anchors = DIRS.map((dir) => {
      const a = document.createElement("div");
      a.className = "ito-anchor";
      a.dataset.dir = dir;
      el.appendChild(a);
      return a;
    });
    let drag = null;

    const get = () => ({ x: el.offsetLeft, y: el.offsetTop, w: el.offsetWidth, h: el.offsetHeight });

    function fit(r) {
      const vw = root.innerWidth, vh = root.innerHeight;
      const w = clamp(Math.round(r.w), minW, Math.max(minW, vw));
      const h = clamp(Math.round(r.h), minH, Math.max(minH, vh));
      return { x: clamp(Math.round(r.x), 0, Math.max(0, vw - w)), y: clamp(Math.round(r.y), 0, Math.max(0, vh - h)), w, h };
    }

    function set(r) {
      const f = fit(r);
      Object.assign(el.style, { left: `${f.x}px`, top: `${f.y}px`, width: `${f.w}px`, height: `${f.h}px` });
      if (readout) readout.textContent = `${f.w} × ${f.h}`;
      onChange && onChange(f);
      return f;
    }

    function begin(e, mode, target) {
      if (e.button !== 0 || el.classList.contains("is-locked")) return;
      e.preventDefault();
      drag = { mode, x: e.clientX, y: e.clientY, r: get(), id: e.pointerId, target };
      target.setPointerCapture(e.pointerId);
      target.classList.add("is-active");
      el.classList.add(mode === "move" ? "is-moving" : "is-resizing");
    }

    function move(e) {
      if (!drag || e.pointerId !== drag.id) return;
      const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
      const r0 = drag.r, vw = root.innerWidth, vh = root.innerHeight;
      let { x, y, w, h } = r0;
      if (drag.mode === "move") {
        x = r0.x + dx;
        y = r0.y + dy;
      } else {
        const d = drag.mode;
        if (d.includes("e")) w = clamp(r0.w + dx, minW, vw - r0.x);
        if (d.includes("s")) h = clamp(r0.h + dy, minH, vh - r0.y);
        if (d.includes("w")) { x = clamp(r0.x + dx, 0, r0.x + r0.w - minW); w = r0.x + r0.w - x; }
        if (d.includes("n")) { y = clamp(r0.y + dy, 0, r0.y + r0.h - minH); h = r0.y + r0.h - y; }
      }
      set({ x, y, w, h });
    }

    function end(e) {
      if (!drag || e.pointerId !== drag.id) return;
      drag.target.classList.remove("is-active");
      el.classList.remove("is-moving", "is-resizing");
      drag = null;
      onCommit && onCommit(get());
    }

    for (const a of anchors) {
      a.addEventListener("pointerdown", (e) => begin(e, a.dataset.dir, a));
    }
    handle.addEventListener("pointerdown", (e) => {
      if (!e.target.closest("button")) begin(e, "move", handle);
    });
    for (const t of [...anchors, handle]) {
      t.addEventListener("pointermove", move);
      t.addEventListener("pointerup", end);
      t.addEventListener("pointercancel", end);
    }

    return { get, set };
  };
})(typeof window !== "undefined" ? window : globalThis);
