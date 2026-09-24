// Rolling buffer of live samples plus the brake events (and their peaks) inside it.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  class LiveTrace {
    constructor() {
      this.clear();
    }

    clear() {
      this.t = []; // session seconds
      this.D = []; // cumulative metres (lap × track length + lap distance)
      this.b = [];
      this.th = [];
      this.head = 0; // first live index; older entries await compaction
      this.events = []; // { peak, peakT, peakD, active, onT, onD, onLap, onPct, offT }
      this.current = null;
      this.last = null;
    }

    push(s) {
      this.t.push(s.t);
      this.D.push(s.D);
      this.b.push(s.brake);
      this.th.push(s.throttle);
      this.last = s;

      const ev = this.current;
      if (!ev) {
        if (s.brake > ITO.BRAKE_ON) {
          // Where the brake went on: the brake cue grades it against the reference.
          this.current = { peak: s.brake, peakT: s.t, peakD: s.D, active: true, onT: s.t, onD: s.D, onLap: s.lap, onPct: s.pct };
          this.events.push(this.current);
        }
      } else {
        if (s.brake > ev.peak) Object.assign(ev, { peak: s.brake, peakT: s.t, peakD: s.D });
        if (s.brake < ITO.BRAKE_OFF) { ev.active = false; ev.offT = s.t; this.current = null; }
      }
    }

    // Forget samples older than both limits (the widest window either axis can show).
    prune(minT, minD) {
      let h = this.head;
      while (h < this.t.length - 2 && this.t[h] < minT && this.D[h] < minD) h++;
      this.head = h;
      if (h > 4096) {
        for (const k of ["t", "D", "b", "th"]) this[k] = this[k].slice(h);
        this.head = 0;
      }
      this.events = this.events.filter((e) => e.active || e.peakT >= minT || e.peakD >= minD);
    }
  }

  ITO.LiveTrace = LiveTrace;
})(typeof window !== "undefined" ? window : globalThis);
