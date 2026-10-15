// "From pack to pixels": seven thousand points that travel through four shapes as you scroll.
// A packfile (a loose cloud), decoded commits (a tight stream), a laid-out graph (coloured lanes),
// and the screen (a grid where only the visible rows light up). Three.js draws them as one Points mesh;
// a vertex shader mixes between the four target positions, so the CPU does nothing per frame.
(() => {
  const section = document.getElementById('flow');
  const canvas = document.getElementById('particles');
  if (!section || !canvas || typeof THREE === 'undefined') return;
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;

  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ canvas, antialias: false, alpha: true, powerPreference: 'high-performance' });
  } catch (e) { return; }
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.setClearColor(0x000000, 0);

  const hexToRgb = (hex) => {
    const n = parseInt(hex.replace('#', ''), 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
  };
  const accent = () => hexToRgb(getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#b5122f');

  // ---- the shapes: a cloud, a stream, a graph, a screen ----
  const N = 7000;
  let seed = 15293442;
  const rnd = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296; };
  const pA = new Float32Array(N * 3), pB = new Float32Array(N * 3), pC = new Float32Array(N * 3), pD = new Float32Array(N * 3);
  const cC = new Float32Array(N * 3), lit = new Float32Array(N), sz = new Float32Array(N), ph = new Float32Array(N);
  const laneCol = [[0.42, 0.78, 0.86], [0.77, 0.54, 0.84], [0.61, 0.80, 0.48], [0.85, 0.70, 0.43], [0.95, 0.60, 0.62], [0.49, 0.72, 1.0]];
  const COLS = 140, ROWS = Math.ceil(N / COLS);

  // the graph: a trunk and branches that fork off, run, and merge back; each is a segment list
  // so points are spread by length and no branch has holes
  const TOP = 1.2, BOT = -1.2, STEP = 0.6, TRUNK = -1.5;
  const segs = [{ lane: 0, y0: TOP, y1: BOT, from: 0, to: 0 }];
  let cursor = TOP - 0.05;
  const lanesBusy = [];
  for (let k = 0; k < 11; k++) {
    const len = 0.5 + rnd() * 1.1;
    const y0 = cursor, y1 = Math.max(BOT + 0.05, y0 - len);
    let lane = 1 + Math.floor(rnd() * 5);
    for (let tries = 0; tries < 5 && lanesBusy.some((b) => b.lane === lane && !(b.y1 > y0 || b.y0 < y1)); tries++) lane = 1 + Math.floor(rnd() * 5);
    lanesBusy.push({ lane, y0, y1 });
    // branches either fork from the trunk or from the previous branch, and merge back to the trunk or the next one
    const prev = segs[segs.length - 1];
    const from = rnd() < 0.3 && prev.lane > 0 && prev.y0 > y0 && prev.y1 < y0 ? prev.lane : 0;
    const to = rnd() < 0.3 && prev.lane > 0 && prev.y1 < y1 ? prev.lane : 0;
    segs.push({ lane, y0, y1, from, to });
    cursor -= 0.14 + rnd() * 0.22;
    if (y1 <= BOT + 0.05) break;
    if (cursor < BOT + 0.4) cursor = TOP - 0.05 - rnd() * 0.3;
  }
  const total = segs.reduce((a, g) => a + (g.y0 - g.y1), 0);
  const ease = (x) => x * x * (3 - 2 * x);
  for (let i = 0; i < N; i++) {
    // A: a loose cloud, denser at the centre, with a few far strays
    const r = Math.pow(rnd(), 0.75) * (rnd() < 0.08 ? 3.2 : 1.9), th = rnd() * Math.PI * 2, u = rnd() * 2 - 1;
    const sq = Math.sqrt(1 - u * u);
    pA[i * 3] = r * sq * Math.cos(th) * 1.5; pA[i * 3 + 1] = r * u * 0.65; pA[i * 3 + 2] = r * sq * Math.sin(th);
    // B: a stream, ordered by index, twisting along x
    const t = i / N;
    const x = -3.3 + t * 6.6;
    const tw = Math.sin(t * 7.0) * 0.38 + Math.sin(t * 19.0) * 0.08;
    const ang = rnd() * Math.PI * 2, rr = Math.pow(rnd(), 0.5) * 0.2;
    pB[i * 3] = x; pB[i * 3 + 1] = tw + Math.cos(ang) * rr; pB[i * 3 + 2] = Math.sin(ang) * rr * 3.0;
    // C: a place on the graph, by length along the segments
    let d = t * total, g = segs[0];
    for (const sg of segs) { const l = sg.y0 - sg.y1; if (d <= l) { g = sg; break; } d -= l; }
    const y = g.y0 - d;
    const l = g.y0 - g.y1;
    const R = Math.min(0.3, l * 0.35);
    const a0 = ease(Math.min(1, d / R)), a1 = ease(Math.min(1, (l - d) / R));
    const xFrom = TRUNK + g.from * STEP, xTo = TRUNK + g.to * STEP, xLane = TRUNK + g.lane * STEP;
    const xx = g.lane === 0 ? xLane : (d < l / 2 ? xFrom + (xLane - xFrom) * a0 : xTo + (xLane - xTo) * a1);
    const jitter = (rnd() - 0.5) * 0.012;
    pC[i * 3] = xx + jitter; pC[i * 3 + 1] = y; pC[i * 3 + 2] = (rnd() - 0.5) * 0.02;
    cC[i * 3] = laneCol[g.lane][0]; cC[i * 3 + 1] = laneCol[g.lane][1]; cC[i * 3 + 2] = laneCol[g.lane][2];
    // D: a screen grid; the visible rows are a band in the middle
    const col = i % COLS, row = Math.floor(i / COLS);
    pD[i * 3] = -2.9 + (col / (COLS - 1)) * 5.8; pD[i * 3 + 1] = 1.45 - (row / (ROWS - 1)) * 2.9; pD[i * 3 + 2] = 0;
    const mid = Math.abs(row - ROWS * 0.5);
    lit[i] = mid < ROWS * 0.17 ? 1 : 0;
    sz[i] = 0.7 + rnd() * 0.8;
    ph[i] = rnd() * 6.283;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(pA, 3));
  geo.setAttribute('pB', new THREE.BufferAttribute(pB, 3));
  geo.setAttribute('pC', new THREE.BufferAttribute(pC, 3));
  geo.setAttribute('pD', new THREE.BufferAttribute(pD, 3));
  geo.setAttribute('cC', new THREE.BufferAttribute(cC, 3));
  geo.setAttribute('lit', new THREE.BufferAttribute(lit, 1));
  geo.setAttribute('sz', new THREE.BufferAttribute(sz, 1));
  geo.setAttribute('ph', new THREE.BufferAttribute(ph, 1));

  const uniforms = {
    uPhase: { value: 0 }, uTime: { value: 0 }, uPx: { value: 1 },
    uColor: { value: new THREE.Vector3(...accent()) }, uMouse: { value: new THREE.Vector2(0, 0) },
  };
  const vert = `
    attribute vec3 pB; attribute vec3 pC; attribute vec3 pD; attribute vec3 cC;
    attribute float lit; attribute float sz; attribute float ph;
    uniform float uPhase; uniform float uTime; uniform float uPx; uniform vec3 uColor;
    varying vec3 vCol; varying float vA;
    float ease(float x) { return x * x * (3.0 - 2.0 * x); }
    void main() {
      // each point leaves a little late or early, so the shapes dissolve instead of snapping
      float d = fract(ph * 0.159) * 0.35;
      float sCol = ease(clamp((uPhase - d * 2.0) / (1.0 - d * 0.5), 0.0, 1.0));          // 0 -> 1: colour lands on each pixel
      float s0 = ease(clamp((uPhase - 1.0 - d * 0.6) / (1.0 - d * 0.6), 0.0, 1.0));     // 1 -> 2: into the stream
      float s1 = ease(clamp((uPhase - 2.0 - d) / (1.0 - d), 0.0, 1.0));                 // 2 -> 3: onto the graph
      float s2 = ease(clamp((uPhase - 3.0 - d) / (1.0 - d), 0.0, 1.0));                 // 3 -> 4: onto the screen
      vec3 p = position;
      p += vec3(sin(uTime * 0.6 + ph), cos(uTime * 0.5 + ph * 1.3), sin(uTime * 0.4 + ph * 0.7)) * 0.06 * (1.0 - s0);
      // a coloured pixel twitches once when it is decoded
      p += vec3(0.0, 0.05, 0.0) * sin(sCol * 3.1416) * (1.0 - s0);
      p = mix(p, pB, s0);
      p.z += sin(s0 * 3.1416) * (fract(ph) - 0.5) * 1.6 * (1.0 - s1);
      p.x += (1.0 - s1) * s0 * sin(uTime * 2.0 + ph * 4.0) * 0.02;
      p = mix(p, pC, s1);
      p = mix(p, pD, s2);
      p.y += s2 * lit * sin(uTime * 3.0 + ph) * 0.004;
      vec4 mv = modelViewMatrix * vec4(p, 1.0);
      gl_Position = projectionMatrix * mv;
      float size = sz * (1.4 - 0.5 * s0) * mix(1.0, 1.9, s2 * lit) * mix(1.0, 0.8, s2 * (1.0 - lit)) * mix(1.0, 1.2, s1 * (1.0 - s2)) * mix(1.0, 1.3, sCol * (1.0 - s0));
      gl_PointSize = size * 3.2 * uPx * (6.0 / max(1.0, -mv.z));
      vec3 dim = vec3(0.55, 0.48, 0.56);
      vec3 c = mix(dim, cC, sCol);
      vec3 warm = mix(uColor, vec3(1.0), 0.35);
      c = mix(c, mix(warm, cC, 0.35), s0);
      c = mix(c, cC, s1);
      // the screen: tints of the accent, with each pixel keeping a hint of its own colour
      float tone = 0.6 + 0.6 * fract(ph * 0.73);
      vec3 screen = mix(vec3(0.22, 0.18, 0.21), mix(uColor * tone, cC, 0.3) + vec3(0.12) * fract(ph * 1.7), lit);
      c = mix(c, screen, s2);
      vCol = c;
      vA = mix(0.5, 0.45, s0) * mix(1.0, 0.9, s1) * mix(1.0, mix(0.4, 1.7, lit), s2) * mix(1.0, 1.0, sCol * (1.0 - s0));
    }`;
  const frag = `
    precision highp float; varying vec3 vCol; varying float vA;
    void main() {
      vec2 q = gl_PointCoord - 0.5; float r = length(q);
      float a = smoothstep(0.5, 0.32, r) * vA;
      gl_FragColor = vec4(vCol * a, a);
    }`;
  const mat = new THREE.ShaderMaterial({ uniforms, vertexShader: vert, fragmentShader: frag, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending });
  const scene = new THREE.Scene();
  scene.add(new THREE.Points(geo, mat));
  scene.position.y = -0.3;
  const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 50);
  camera.position.set(0, 0, 6);

  const stick = canvas.parentElement;
  const resize = () => {
    const w = stick.clientWidth, h = stick.clientHeight;
    renderer.setSize(w, h, false);
    camera.aspect = w / h; camera.updateProjectionMatrix();
    uniforms.uPx.value = renderer.getPixelRatio();
    // narrow screens: step back so the shapes fit
    camera.position.z = w < h ? 9 : 6;
  };
  resize();
  addEventListener('resize', resize);
  section.classList.add('gl');

  // ---- scroll drives the phase ----
  const steps = [...document.querySelectorAll('#flow-steps li')];
  let phase = 0, target = 0, current = -1;
  const onScroll = () => {
    const r = section.getBoundingClientRect();
    const travel = r.height - innerHeight;
    const p = Math.min(1, Math.max(0, -r.top / Math.max(1, travel)));
    // the last stretch of the section is a hold, so the screen is seen before the page moves on
    const q = Math.min(1, p / 0.86);
    const seg = q * 4.0;
    const k = Math.floor(seg), f = seg - k;
    const hold = 0.25;
    target = k >= 4 ? 4 : k + Math.max(0, (f - hold) / (1 - hold));
    const step = Math.min(3, target < 0.5 ? 0 : target < 2.5 ? 1 : target < 3.5 ? 2 : 3);
    if (step !== current) { current = step; steps.forEach((li, i) => li.classList.toggle('is-on', i === step)); }
    section.classList.toggle('is-done', p > 0.95);
  };
  addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  let mouse = [0, 0], mt = [0, 0];
  addEventListener('pointermove', (e) => { mt = [(e.clientX / innerWidth - 0.5) * 2, -(e.clientY / innerHeight - 0.5) * 2]; }, { passive: true });
  new MutationObserver(() => uniforms.uColor.value.set(...accent())).observe(document.documentElement, { attributes: true, attributeFilter: ['style'] });

  let visible = false;
  new IntersectionObserver(([e]) => { visible = e.isIntersecting; }).observe(section);
  const start = performance.now();
  const tick = () => {
    requestAnimationFrame(tick);
    if (!visible || document.hidden) return;
    phase += (target - phase) * 0.08;
    mouse[0] += (mt[0] - mouse[0]) * 0.05; mouse[1] += (mt[1] - mouse[1]) * 0.05;
    uniforms.uPhase.value = phase;
    uniforms.uTime.value = (performance.now() - start) / 1000;
    // the camera orbits a little with the pointer and settles square-on for the screen
    const flat = Math.max(0, phase - 3);
    scene.rotation.y = mouse[0] * 0.25 * (1 - flat) + Math.sin(uniforms.uTime.value * 0.15) * 0.08 * (1 - flat);
    scene.rotation.x = -mouse[1] * 0.15 * (1 - flat);
    renderer.render(scene, camera);
  };
  tick();
})();
