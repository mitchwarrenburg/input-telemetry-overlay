// Stand-in for iRacing live telemetry while the overlay is being designed.
// Replays a lap's position and varies the driver's inputs per brake zone, so the
// live lines differ from the reference the way a real attempt would.
(function (root) {
  const ITO = (root.ITO = root.ITO || {});

  function mulberry32(seed) {
    return function () {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  function sampleAt(arr, f) {
    const n = arr.length;
    if (f <= 0) return arr[0];
    if (f >= n - 1) return arr[n - 1];
    const i = Math.floor(f);
    return arr[i] + (arr[i + 1] - arr[i]) * (f - i);
  }

  class SimulatedDriver {
    constructor(lap, trackLength, seed = 11) {
      this.lap = lap;
      this.L = trackLength;
      this.rand = mulberry32(seed);
      this.t = 0;
      this.lapIndex = 0;
      this.lapStart = 0;
      this.vary();
    }

    // New set of inputs for the coming lap.
    vary() {
      const { lap, rand } = this;
      const n = lap.n;
      const r = (a, b) => a + (b - a) * rand();
      const brake = new Float32Array(n);
      const throttle = Float32Array.from(lap.throttle);
      const zones = lap.zones;

      zones.forEach((z, k) => {
        const lo = k ? Math.floor((zones[k - 1].end + z.start) / 2) : 0;
        const hi = k < zones.length - 1 ? Math.floor((z.end + zones[k + 1].start) / 2) : n - 1;
        const shift = r(-9, 7); // samples: brake earlier (−) or later (+)
        const stretch = r(0.9, 1.15); // longer or shorter trail
        const scale = r(0.88, 1.3); // softer or harder pedal
        const spike = rand() < 0.5 ? r(0.04, 0.12) : 0; // initial stab
        const spikeAt = z.start + shift + 7;
        const lag = r(-3, 10); // throttle pickup earlier (−) or later (+)
        for (let j = lo; j <= hi; j++) {
          let b = sampleAt(lap.brake, z.start + (j - shift - z.start) / stretch) * scale;
          if (spike) b += spike * Math.exp(-(((j - spikeAt) / 5) ** 2));
          brake[j] = Math.min(1, Math.max(0, b));
          throttle[j] = brake[j] > 0.05 ? 0 : sampleAt(lap.throttle, j - lag);
        }
      });

      // Now and then, a lift on a straight.
      if (rand() < 0.6) {
        for (let tries = 0; tries < 30; tries++) {
          const c = Math.floor(r(0, n - 120));
          if (throttle.subarray(c, c + 90).some((v) => v < 0.99)) continue;
          const depth = r(0.2, 0.45);
          for (let j = c; j < c + 90; j++) throttle[j] = Math.min(throttle[j], 1 - depth * Math.exp(-(((j - c - 45) / 12) ** 2)));
          break;
        }
      }
      this.brake = brake;
      this.throttle = throttle;
    }

    // Advance by dt seconds and return one telemetry sample.
    step(dt) {
      const lap = this.lap;
      this.t += dt;
      let tl = this.t - this.lapStart;
      while (tl >= lap.lapTime) {
        this.lapStart += lap.lapTime;
        tl -= lap.lapTime;
        this.lapIndex++;
        this.vary();
      }
      const f = tl * lap.hz;
      const i = Math.min(lap.n - 1, Math.floor(f));
      const p0 = lap.pct[i];
      const p1 = i + 1 < lap.n ? lap.pct[i + 1] : lap.pct[0] + 1; // runs past 1 into the next lap
      const pctRun = p0 + (p1 - p0) * (f - i);
      return {
        t: this.t,
        lap: this.lapIndex + Math.floor(pctRun),
        pct: pctRun % 1,
        D: (this.lapIndex + pctRun) * this.L, // cumulative metres, continuous across S/F
        brake: sampleAt(this.brake, f),
        throttle: sampleAt(this.throttle, f),
      };
    }
  }

  ITO.SimulatedDriver = SimulatedDriver;
})(typeof window !== "undefined" ? window : globalThis);
