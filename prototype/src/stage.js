// Prototype only: a stylised track view behind the overlay, so background
// opacity can be judged against something that looks like the sim.
(function () {
  const W = 1600, H = 900, HZN = 440;
  const lerp = (a, b, t) => a + (b - a) * t;
  const pt = (x0, x1, t) => `${lerp(x0, x1, t).toFixed(1)},${lerp(HZN, H, t).toFixed(1)}`;
  // Equal ground spacing → shrinking screen spacing toward the horizon.
  const ts = Array.from({ length: 48 }, (_, k) => 1 / (1 + k * 0.32));

  const vL = 860, vR = 900; // road edges at the horizon
  const bL = -120, bR = 1760; // road edges at the bottom of the screen

  function kerb(inV, inB, outV, outB) {
    let s = "";
    for (let k = 0; k < ts.length - 1; k++) {
      const a = ts[k], b = ts[k + 1];
      const fog = 1 - a;
      const fill = k % 2 ? `rgba(226,226,222,${0.85 - fog * 0.6})` : `rgba(196,48,40,${0.9 - fog * 0.6})`;
      s += `<polygon fill="${fill}" points="${pt(inV, inB, a)} ${pt(inV, inB, b)} ${pt(outV, outB, b)} ${pt(outV, outB, a)}"/>`;
    }
    return s;
  }

  const svg = `
<svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="xMidYMid slice" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#101b27"/><stop offset="0.6" stop-color="#3a5064"/><stop offset="1" stop-color="#7d93a0"/>
    </linearGradient>
    <radialGradient id="sun" cx="0.62" cy="0.47" r="0.35">
      <stop offset="0" stop-color="#f3d9a4" stop-opacity="0.55"/><stop offset="1" stop-color="#f3d9a4" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="grass" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#34502a"/><stop offset="1" stop-color="#16240f"/>
    </linearGradient>
    <linearGradient id="road" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#5c6166"/><stop offset="1" stop-color="#26292c"/>
    </linearGradient>
    <radialGradient id="vignette" cx="0.5" cy="0.5" r="0.75">
      <stop offset="0.55" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.6"/>
    </radialGradient>
  </defs>
  <rect width="${W}" height="${HZN + 2}" fill="url(#sky)"/>
  <rect width="${W}" height="${H}" fill="url(#sun)"/>
  <path fill="#243542" d="M0 ${HZN - 26} C 220 ${HZN - 60} 420 ${HZN - 18} 640 ${HZN - 38} S 1080 ${HZN - 70} 1320 ${HZN - 30} S 1560 ${HZN - 40} ${W} ${HZN - 34} V ${HZN} H 0 Z"/>
  <path fill="#15221a" d="M0 ${HZN - 10} q 30 -16 60 -4 t 60 -8 t 70 2 t 60 -10 t 80 6 t 70 -12 t 80 8 t 90 -6 t 70 -4 t 90 10 t 80 -14 t 90 8 t 80 -6 t 90 4 t 70 -10 t 90 6 t 80 -8 t 90 10 t 60 -6 V ${HZN + 2} H 0 Z"/>
  <rect x="1040" y="${HZN - 22}" width="170" height="20" fill="#2a343c"/>
  <rect x="1046" y="${HZN - 30}" width="158" height="9" fill="#3b4751"/>
  <rect y="${HZN}" width="${W}" height="${H - HZN}" fill="url(#grass)"/>
  <polygon fill="url(#road)" points="${vL},${HZN} ${vR},${HZN} ${bR},${H} ${bL},${H}"/>
  <polygon fill="rgba(255,255,255,0.5)" points="${pt(vL, bL, 0)} ${pt(vL + 1, bL + 26, 0)} ${pt(vL + 1, bL + 26, 1)} ${pt(vL, bL, 1)}"/>
  <polygon fill="rgba(255,255,255,0.5)" points="${pt(vR, bR, 0)} ${pt(vR - 1, bR - 26, 0)} ${pt(vR - 1, bR - 26, 1)} ${pt(vR, bR, 1)}"/>
  ${kerb(vL, bL, vL - 5, bL - 150)}
  ${kerb(vR, bR, vR + 5, bR + 150)}
  <rect width="${W}" height="${H}" fill="url(#vignette)"/>
</svg>`;
  document.getElementById("stage").innerHTML = svg;
})();
