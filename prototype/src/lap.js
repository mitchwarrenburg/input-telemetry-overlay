// Reference lap model + Garage 61 CSV parser.
// A lap is indexed by sample; LapDistPct is the key that lines it up with the live car.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  const REQUIRED = ["LapDistPct", "Brake", "Throttle"];
  const BRAKE_ON = 0.05; // brake event starts above this…
  const BRAKE_OFF = 0.02; // …and ends below this (hysteresis)
  const TELEMETRY_HZ = 60; // iRacing's disk/live telemetry rate
  // Auto-blips (as src/lap.rs). With a Gear column, blips are found around each downshift;
  // without, a pulse off BLIP_FLOOR that jumps BLIP_JUMP at once and is gone in BLIP_MAX_S.
  const BLIP_MAX_S = 0.2;
  const BLIP_FLOOR = 0.05;
  const BLIP_JUMP = 0.15;
  const BLIP_RISE = 0.05; // per sample while a blip builds
  const BLIP_LEAD = 2; // samples a blip may start before the gear reads neutral
  const BLIP_DECAY_S = 0.2; // after the new gear engages

  // "Garage 61 - Driver - Car - Track - 01.55.992 - ID.csv"
  function parseFileName(fileName) {
    const base = String(fileName || "").replace(/^.*[\\/]/, "").replace(/\.csv$/i, "");
    const parts = base.split(" - ");
    const meta = { fileName: base, source: null, driver: null, car: null, track: null, lapTime: null };
    const timeIdx = parts.findIndex((p) => /^\d{1,2}\.\d{2}\.\d{3}$/.test(p));
    if (parts[0] === "Garage 61" && timeIdx >= 4) {
      meta.source = "Garage 61";
      meta.driver = parts[1];
      meta.car = parts[2];
      meta.track = parts.slice(3, timeIdx).join(" - ");
      const [m, s, ms] = parts[timeIdx].split(".").map(Number);
      meta.lapTime = m * 60 + s + ms / 1000;
    }
    return meta;
  }

  function formatLapTime(sec) {
    if (!isFinite(sec)) return "–";
    const m = Math.floor(sec / 60);
    const s = sec - m * 60;
    return `${m}:${s.toFixed(3).padStart(6, "0")}`;
  }

  // A straight line from sample `from` to sample `to`, replacing those between.
  function bridge(v, from, to) {
    const a = v[from], b = v[to], span = to - from;
    for (let k = from + 1; k < to; k++) v[k] = a + ((b - a) * (k - from)) / span;
  }

  // Garage 61 exports iRacing's Throttle, where the car blips the throttle on every
  // downshift; the live line reads the pedal, which doesn't. Flattens them. Returns the count.
  function removeBlips(thr, gear, hz) {
    let removed = 0;
    if (gear && gear.length === thr.length && gear.includes(0)) {
      const n = thr.length, decay = Math.round(BLIP_DECAY_S * hz);
      for (let i = 1; i < n; ) {
        if (gear[i] !== 0 || gear[i - 1] <= 0) { i++; continue; }
        const neutral = i;
        while (i < n && gear[i] === 0) i++;
        const engaged = i;
        if (engaged === n || gear[engaged] <= 0 || gear[engaged] >= gear[neutral - 1]) continue; // upshift
        let start = neutral;
        while (start > 1 && neutral - start < BLIP_LEAD && thr[start - 1] > thr[start - 2] + BLIP_RISE) start--;
        let end = engaged;
        while (end + 1 < n && end < engaged + decay && thr[end + 1] < thr[end] - 0.01) end++;
        const before = thr[start - 1];
        if (!thr.slice(start, end).some((v) => v > before + 0.02)) continue;
        bridge(thr, start - 1, end);
        removed++;
      }
      return removed;
    }
    const longest = Math.max(1, Math.round(BLIP_MAX_S * hz));
    for (let i = 1; i < thr.length; i++) {
      if (thr[i] > BLIP_FLOOR && thr[i - 1] <= BLIP_FLOOR) {
        let end = i;
        while (end < thr.length && thr[end] > BLIP_FLOOR) end++;
        if (end === thr.length) break;
        if (end - i <= longest && thr[i] - thr[i - 1] >= BLIP_JUMP) { bridge(thr, i - 1, end); removed++; }
        i = end;
      }
    }
    return removed;
  }

  function findZones(brake) {
    const zones = [];
    let z = null;
    for (let i = 0; i < brake.length; i++) {
      const b = brake[i];
      if (!z) {
        if (b > BRAKE_ON) z = { start: i, end: i, peakIdx: i, peak: b };
      } else {
        if (b > z.peak) { z.peak = b; z.peakIdx = i; }
        if (b < BRAKE_OFF) { z.end = i; zones.push(z); z = null; }
      }
    }
    if (z) { z.end = brake.length - 1; zones.push(z); }
    return zones;
  }

  function parseLapCsv(text, fileName) {
    const lines = String(text).replace(/^﻿/, "").split(/\r?\n/);
    const header = (lines[0] || "").split(",").map((h) => h.trim());
    const col = (name) => header.findIndex((h) => h.toLowerCase() === name.toLowerCase());
    const missing = REQUIRED.filter((c) => col(c) < 0);
    if (missing.length) {
      throw new Error(`Missing column${missing.length > 1 ? "s" : ""}: ${missing.join(", ")}. Export the lap from Garage 61 as CSV.`);
    }
    const iPct = col("LapDistPct"), iBrake = col("Brake"), iThr = col("Throttle"), iSpd = col("Speed"), iGear = col("Gear");

    let pct = [], brake = [], throttle = [], speed = [], gear = [];
    for (let r = 1; r < lines.length; r++) {
      if (!lines[r]) continue;
      const f = lines[r].split(",");
      const p = parseFloat(f[iPct]), b = parseFloat(f[iBrake]), t = parseFloat(f[iThr]);
      if (!isFinite(p) || !isFinite(b) || !isFinite(t)) continue;
      pct.push(p);
      brake.push(Math.min(1, Math.max(0, b)));
      throttle.push(Math.min(1, Math.max(0, t)));
      speed.push(iSpd >= 0 ? parseFloat(f[iSpd]) || 0 : 0);
      gear.push(iGear >= 0 ? parseInt(f[iGear], 10) || 0 : 0);
    }

    // Exports often carry a sample or two from the neighbouring lap: drop whichever
    // side of the start/finish wrap is the short one.
    for (let i = 1; i < pct.length; i++) {
      if (pct[i] < pct[i - 1] - 0.5) {
        const keep = i > pct.length / 2 ? [0, i] : [i, pct.length];
        [pct, brake, throttle, speed, gear] = [pct, brake, throttle, speed, gear].map((a) => a.slice(keep[0], keep[1]));
        i = 0;
      }
    }
    if (pct.length < TELEMETRY_HZ * 10) throw new Error("That file holds less than 10 seconds of driving — is it a full lap?");
    for (let i = 1; i < pct.length; i++) if (pct[i] < pct[i - 1]) pct[i] = pct[i - 1]; // float jitter

    const meta = parseFileName(fileName);
    const n = pct.length;
    // No time column in the export: derive the rate from the lap time in the file name.
    let hz = meta.lapTime ? n / meta.lapTime : TELEMETRY_HZ;
    if (hz < 20 || hz > 400) hz = TELEMETRY_HZ;
    const lapTime = meta.lapTime || n / hz;
    const blips = removeBlips(throttle, iGear >= 0 ? gear : null, hz);

    let trackLengthEst = null;
    if (iSpd >= 0) {
      trackLengthEst = 0;
      for (let i = 0; i < n; i++) trackLengthEst += speed[i] / hz;
    }

    const lap = {
      meta,
      n,
      hz,
      lapTime,
      lapTimeText: formatLapTime(lapTime),
      blips,
      trackLengthEst,
      pct: Float64Array.from(pct),
      brake: Float32Array.from(brake),
      throttle: Float32Array.from(throttle),
      speed: iSpd >= 0 ? Float32Array.from(speed) : null,
      zones: findZones(brake),
    };

    // Fractional sample index at a lap fraction. Values before the first sample
    // resolve against the previous lap's tail (negative index), so the result
    // is continuous across start/finish.
    lap.indexAtPct = function (p) {
      const a = lap.pct;
      if (p < a[0]) {
        const prev = a[n - 1] - 1;
        return -1 + (p - prev) / Math.max(1e-9, a[0] - prev);
      }
      if (p >= a[n - 1]) {
        const next = a[0] + 1;
        return n - 1 + (p - a[n - 1]) / Math.max(1e-9, next - a[n - 1]);
      }
      let lo = 0, hi = n - 1;
      while (hi - lo > 1) {
        const mid = (lo + hi) >> 1;
        if (a[mid] <= p) lo = mid; else hi = mid;
      }
      return lo + (p - a[lo]) / Math.max(1e-9, a[hi] - a[lo]);
    };

    // First sample index at or after a lap fraction (for windowing).
    lap.lowerBound = function (p) {
      let lo = 0, hi = n;
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (lap.pct[mid] < p) lo = mid + 1; else hi = mid;
      }
      return lo;
    };

    return lap;
  }

  // Four-column copy of a lap, small enough to persist in localStorage.
  function serializeLap(lap) {
    const rows = ["LapDistPct,Brake,Throttle,Speed"];
    for (let i = 0; i < lap.n; i++) {
      rows.push(`${+lap.pct[i].toFixed(7)},${+lap.brake[i].toFixed(4)},${+lap.throttle[i].toFixed(4)},${lap.speed ? +lap.speed[i].toFixed(2) : 0}`);
    }
    return rows.join("\n");
  }

  Object.assign(ITO, { parseLapCsv, parseFileName, formatLapTime, serializeLap, BRAKE_ON, BRAKE_OFF });
})(typeof window !== "undefined" ? window : globalThis);
