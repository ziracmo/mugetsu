// The releases page: reads ziracmo/mugetsu releases from GitHub, gives each one a scene drawn
// from the feature that defines it (or a "fixes and polish" scene), and lists the notes.
(() => {
  const M = window.Motion;
  const REPO = 'ziracmo/mugetsu';
  const list = document.getElementById('releases');
  const tpl = document.getElementById('tpl-release');
  const count = document.getElementById('rel-count');
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ---- scenes: shapes in a 3.2 x 2.4 box, sampled into points ----
  const LANE = { cyan: [0.42, 0.78, 0.86], purple: [0.77, 0.54, 0.84], green: [0.61, 0.80, 0.48], gold: [0.85, 0.70, 0.43], blue: [0.49, 0.72, 1.0] };
  const GREY = [0.42, 0.38, 0.42], DIM = [0.28, 0.25, 0.29], WHITE = [0.95, 0.92, 0.93], RED = [0.88, 0.42, 0.46];
  const A = 'accent'; // resolved to the page accent at draw time
  const line = (a, b, col, w = 1) => ({ k: 'line', a, b, col, w });
  const ring = (c, r, col, w = 1) => ({ k: 'ring', c, r, col, w });
  const disc = (c, r, col) => ({ k: 'disc', c, r, col });
  const rect = (x, y, w, h, col) => ({ k: 'rect', x, y, w, h, col });
  const curve = (a, b, col, w = 1) => { // an S between two x at two y
    const out = []; const n = 14;
    for (let i = 0; i < n; i++) {
      const t0 = i / n, t1 = (i + 1) / n, e = (t) => t * t * (3 - 2 * t);
      out.push(line([a[0] + (b[0] - a[0]) * e(t0), a[1] + (b[1] - a[1]) * t0], [a[0] + (b[0] - a[0]) * e(t1), a[1] + (b[1] - a[1]) * t1], col, w));
    }
    return out;
  };
  const rows = (x0, x1, y0, n, gap, col, skip = []) => { const o = []; for (let i = 0; i < n; i++) if (!skip.includes(i)) o.push(line([x0, y0 - i * gap], [x1 - (i * 37 % 5) * 0.08, y0 - i * gap], col, 1.4)); return o; };

  const SCENES = {
    graph: () => [
      line([-0.9, 1.0], [-0.9, -1.0], LANE.cyan, 2), ...curve([-0.9, 0.7], [-0.4, 0.45], LANE.purple, 2), line([-0.4, 0.45], [-0.4, -0.3], LANE.purple, 2), ...curve([-0.4, -0.3], [-0.9, -0.55], LANE.purple, 2),
      ...curve([-0.9, 0.3], [0.1, 0.05], LANE.green, 2), line([0.1, 0.05], [0.1, -0.7], LANE.green, 2), ...curve([0.1, -0.7], [-0.9, -0.95], LANE.green, 2),
      ...curve([-0.4, 0.2], [0.6, -0.05], LANE.gold, 2), line([0.6, -0.05], [0.6, -0.45], LANE.gold, 2), ...curve([0.6, -0.45], [0.1, -0.6], LANE.gold, 2),
      ...[1.0, 0.85, 0.55, 0.3, 0.05, -0.2, -0.45, -0.7, -0.95].map((y) => disc([-0.9, y], 0.045, y === 1.0 ? A : LANE.cyan)),
      disc([-0.4, 0.2], 0.045, LANE.purple), disc([-0.4, -0.1], 0.045, LANE.purple), disc([0.1, -0.2], 0.045, LANE.green), disc([0.1, -0.5], 0.045, LANE.green), disc([0.6, -0.25], 0.045, LANE.gold),
      ...rows(-0.6, 1.5, 1.0, 9, 0.25, DIM),
    ],
    fetch: () => [
      ring([0, 0.05], 0.22, A, 2), ring([0, 0.05], 0.48, [0.6, 0.3, 0.38], 1.2), ring([0, 0.05], 0.74, DIM, 1), ring([0, 0.05], 1.0, DIM, 0.8),
      disc([0, 0.05], 0.09, A), disc([0.72, 0.72], 0.05, A), disc([-0.85, -0.6], 0.04, LANE.cyan), disc([0.95, -0.45], 0.04, LANE.purple),
      rect(-1.45, -1.15, 0.7, 0.14, DIM), rect(-0.65, -1.15, 0.7, 0.14, [0.4, 0.2, 0.26]), rect(0.15, -1.15, 0.7, 0.14, DIM),
    ],
    search: () => [
      ...rows(-1.3, 1.3, 0.85, 8, 0.24, DIM, [3]), line([-1.3, 0.13], [-0.4, 0.13], DIM, 1.4), rect(-0.4, 0.07, 0.75, 0.12, A), line([0.35, 0.13], [1.0, 0.13], DIM, 1.4),
      ring([0.95, 0.55], 0.28, WHITE, 2), line([1.15, 0.35], [1.42, 0.08], WHITE, 3),
    ],
    diff: () => [
      ...rows(-1.1, 1.3, 0.95, 3, 0.24, DIM), rect(-1.4, 0.16, 2.8, 0.17, [0.5, 0.18, 0.22]), line([-1.1, 0.25], [0.9, 0.25], RED, 1.6), rect(-1.4, -0.09, 2.8, 0.17, [0.2, 0.42, 0.22]), line([-1.1, 0.0], [1.0, 0.0], LANE.green, 1.6),
      ...rows(-1.1, 1.3, -0.3, 4, 0.24, DIM), line([-1.3, 0.25], [-1.2, 0.25], RED, 2), line([-1.3, 0.0], [-1.2, 0.0], LANE.green, 2), line([-1.25, 0.05], [-1.25, -0.05], LANE.green, 2),
    ],
    speed: () => [
      rect(-1.4, 0.55, 2.8, 0.2, A), rect(-1.4, 0.15, 0.9, 0.2, DIM), rect(-1.4, -0.25, 1.2, 0.2, DIM), rect(-1.4, -0.65, 1.5, 0.2, DIM),
      disc([1.42, 0.65], 0.07, WHITE), line([-1.45, 0.9], [-1.45, -0.85], GREY, 1),
    ],
    pr: () => [
      ...curve([-1.1, 0.75], [-0.9, 0.75], GREY), line([-1.0, 0.75], [1.0, 0.75], GREY, 1.2), line([1.0, 0.75], [1.0, -0.65], GREY, 1.2), line([1.0, -0.65], [-1.0, -0.65], GREY, 1.2), line([-1.0, -0.65], [-1.0, 0.75], GREY, 1.2),
      rect(-0.8, 0.35, 0.9, 0.2, LANE.green), rect(0.2, 0.35, 0.4, 0.2, A),
      disc([-0.75, -0.05], 0.05, LANE.green), line([-0.6, -0.05], [0.5, -0.05], DIM, 1.4), disc([-0.75, -0.3], 0.05, LANE.green), line([-0.6, -0.3], [0.3, -0.3], DIM, 1.4), disc([-0.75, -0.5], 0.05, LANE.gold), line([-0.6, -0.5], [0.6, -0.5], DIM, 1.4),
    ],
    sidebar: () => [
      line([-1.3, 0.9], [-0.7, 0.9], WHITE, 1.6), rect(-1.1, 0.5, 1.6, 0.17, A), line([-1.05, 0.3], [-0.5, 0.3], GREY, 1.4), line([-1.05, 0.08], [-0.45, 0.08], GREY, 1.4), line([-1.05, -0.14], [-0.6, -0.14], GREY, 1.4),
      line([-1.3, -0.45], [-0.7, -0.45], WHITE, 1.6), line([-1.05, -0.67], [-0.55, -0.67], LANE.blue, 1.4), line([-0.8, -0.89], [-0.3, -0.89], GREY, 1.4),
      line([-1.3, 1.0], [-1.3, -1.0], DIM, 1), ...[0.3, 0.08, -0.14, -0.67].map((y) => rect(-1.22, y - 0.04, 0.09, 0.08, GREY)),
      ...rows(-0.2, 1.4, 0.9, 8, 0.26, DIM),
    ],
    accent: () => [
      disc([0, 0.1], 0.5, A), ring([0, 0.1], 0.62, WHITE, 1.5),
      ...[[0.9, 0.5, 0.2], [0.85, 0.7, 0.3], [0.5, 0.8, 0.4], [0.3, 0.75, 0.9], [0.5, 0.5, 0.95], [0.85, 0.45, 0.8]].map((c, i) => disc([-1.25 + i * 0.5, -0.9], 0.11, c)),
    ],
    eclipse: () => [ring([0, 0.1], 0.72, A, 3), ring([0, 0.1], 0.86, DIM, 1), ring([0, 0.1], 0.98, [0.2, 0.18, 0.2], 0.8), disc([0.95, 0.75], 0.05, WHITE), disc([1.12, 0.52], 0.05, WHITE)],
    fixes: () => {
      // a grid with a few broken cells that settle into place over time (see uGlitch in the shader)
      const o = []; for (let y = 0; y < 7; y++) for (let x = 0; x < 11; x++) o.push(rect(-1.35 + x * 0.26, 0.75 - y * 0.26, 0.12, 0.12, (x * 7 + y * 3) % 9 === 0 ? A : DIM));
      return o;
    },
  };

  const sample = (shapes, N, accent) => {
    const weights = shapes.map((s) => s.k === 'line' ? Math.hypot(s.b[0] - s.a[0], s.b[1] - s.a[1]) * s.w : s.k === 'ring' ? 6.283 * s.r * s.w : s.k === 'disc' ? 3.1416 * s.r * s.r * 28 : s.w * s.h * 28);
    const total = weights.reduce((a, b) => a + b, 0);
    const pos = new Float32Array(N * 3), col = new Float32Array(N * 3), scat = new Float32Array(N * 3), ph = new Float32Array(N);
    let seed = 7;
    const rnd = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 4294967296; };
    let i = 0;
    for (let s = 0; s < shapes.length && i < N; s++) {
      const sh = shapes[s]; const c = sh.col === A ? accent : sh.col;
      const n = s === shapes.length - 1 ? N - i : Math.round((weights[s] / total) * N);
      for (let k = 0; k < n && i < N; k++, i++) {
        let x, y;
        if (sh.k === 'line') { const t = rnd(), j = (rnd() - 0.5) * 0.02 * sh.w; x = sh.a[0] + (sh.b[0] - sh.a[0]) * t; y = sh.a[1] + (sh.b[1] - sh.a[1]) * t + j; x += (rnd() - 0.5) * 0.02 * sh.w; }
        else if (sh.k === 'ring') { const t = rnd() * 6.283, r = sh.r + (rnd() - 0.5) * 0.012 * sh.w; x = sh.c[0] + Math.cos(t) * r; y = sh.c[1] + Math.sin(t) * r; }
        else if (sh.k === 'disc') { const t = rnd() * 6.283, r = Math.sqrt(rnd()) * sh.r; x = sh.c[0] + Math.cos(t) * r; y = sh.c[1] + Math.sin(t) * r; }
        else { x = sh.x + rnd() * sh.w; y = sh.y + rnd() * sh.h; }
        pos[i * 3] = x; pos[i * 3 + 1] = y; pos[i * 3 + 2] = 0;
        col[i * 3] = c[0]; col[i * 3 + 1] = c[1]; col[i * 3 + 2] = c[2];
        const t = rnd() * 6.283, u = rnd() * 2 - 1, r = 1.2 + rnd() * 1.6;
        scat[i * 3] = Math.cos(t) * Math.sqrt(1 - u * u) * r * 1.3; scat[i * 3 + 1] = u * r * 0.8; scat[i * 3 + 2] = Math.sin(t) * Math.sqrt(1 - u * u) * r;
        ph[i] = rnd() * 6.283;
      }
    }
    return { pos, col, scat, ph };
  };

  const hexToRgb = (hex) => { const n = parseInt(hex.replace('#', ''), 16); return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255]; };
  const accent = () => hexToRgb(getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#b5122f');

  const VERT = `
    attribute vec3 scat; attribute vec3 col; attribute float ph;
    uniform float uIn; uniform float uTime; uniform float uPx; uniform float uGlitch; uniform vec2 uMouse;
    varying vec3 vCol; varying float vA;
    float ease(float x) { return x * x * (3.0 - 2.0 * x); }
    void main() {
      float d = fract(ph * 0.159) * 0.4;
      float s = ease(clamp((uIn - d) / (1.0 - d), 0.0, 1.0));
      vec3 p = mix(scat, position, s);
      p += vec3(sin(uTime * 0.7 + ph), cos(uTime * 0.6 + ph * 1.3), 0.0) * 0.012;
      // the fixes scene: a few cells jitter out of place, then snap back, every few seconds
      float burst = smoothstep(0.0, 0.2, fract(uTime * 0.22)) * smoothstep(0.6, 0.3, fract(uTime * 0.22));
      float broken = step(5.6, ph) * uGlitch * burst;
      p.xy += vec2(sin(uTime * 23.0 + ph * 9.0), cos(uTime * 19.0 + ph * 7.0)) * 0.08 * broken;
      // the whole scene tilts toward the pointer
      float cx = cos(uMouse.x * 0.25), sx = sin(uMouse.x * 0.25), cy = cos(uMouse.y * 0.18), sy = sin(uMouse.y * 0.18);
      p = vec3(p.x * cx + p.z * sx, p.y, -p.x * sx + p.z * cx);
      p = vec3(p.x, p.y * cy - p.z * sy, p.y * sy + p.z * cy);
      vec4 mv = modelViewMatrix * vec4(p, 1.0);
      gl_Position = projectionMatrix * mv;
      gl_PointSize = (2.2 + 1.4 * s) * uPx * (4.2 / max(1.0, -mv.z)) * (1.0 + broken * 1.5);
      vCol = mix(vec3(0.5, 0.45, 0.5), col, s) + vec3(0.9, 0.2, 0.3) * broken;
      vA = mix(0.5, 0.95, s);
    }`;
  const FRAG = `precision highp float; varying vec3 vCol; varying float vA;
    void main() { vec2 q = gl_PointCoord - 0.5; float r = length(q); float a = smoothstep(0.5, 0.3, r) * vA; gl_FragColor = vec4(vCol * a, a); }`;

  const scenes = [];
  const mountScene = (canvas, name) => {
    if (typeof THREE === 'undefined' || reduced) { canvas.parentElement.classList.add('is-still'); return; }
    let renderer;
    try { renderer = new THREE.WebGLRenderer({ canvas, antialias: false, alpha: true, powerPreference: 'high-performance' }); } catch (e) { return; }
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);
    const N = 4500;
    const data = sample((SCENES[name] || SCENES.eclipse)(), N, accent());
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(data.pos, 3));
    geo.setAttribute('scat', new THREE.BufferAttribute(data.scat, 3));
    geo.setAttribute('col', new THREE.BufferAttribute(data.col, 3));
    geo.setAttribute('ph', new THREE.BufferAttribute(data.ph, 1));
    const uniforms = { uIn: { value: 0 }, uTime: { value: 0 }, uPx: { value: renderer.getPixelRatio() }, uGlitch: { value: name === 'fixes' ? 1 : 0 }, uMouse: { value: new THREE.Vector2() } };
    const mat = new THREE.ShaderMaterial({ uniforms, vertexShader: VERT, fragmentShader: FRAG, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending });
    const scene = new THREE.Scene(); scene.add(new THREE.Points(geo, mat));
    const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 30); camera.position.z = 4.2;
    const host = canvas.parentElement;
    const resize = () => { const w = host.clientWidth, h = host.clientHeight; renderer.setSize(w, h, false); camera.aspect = w / h; camera.updateProjectionMatrix(); camera.position.z = w < h ? 5.6 : 4.2; };
    resize(); addEventListener('resize', resize);
    const st = { renderer, scene, camera, uniforms, host, visible: false, target: 0, mouse: [0, 0], mt: [0, 0], name, geo, data };
    new IntersectionObserver(([e]) => { st.visible = e.isIntersecting; if (e.isIntersecting) st.target = 1; }, { rootMargin: '0px 0px -15% 0px' }).observe(host);
    host.addEventListener('pointermove', (e) => { const r = host.getBoundingClientRect(); st.mt = [((e.clientX - r.left) / r.width - 0.5) * 2, -((e.clientY - r.top) / r.height - 0.5) * 2]; }, { passive: true });
    host.addEventListener('pointerleave', () => { st.mt = [0, 0]; });
    scenes.push(st);
  };
  // the accent can change from the home page; recolour what pointed at it
  new MutationObserver(() => { const a = accent(); scenes.forEach((st) => { const d = sample((SCENES[st.name] || SCENES.eclipse)(), 4500, a); st.geo.getAttribute('col').array.set(d.col); st.geo.getAttribute('col').needsUpdate = true; }); }).observe(document.documentElement, { attributes: true, attributeFilter: ['style'] });
  const start = performance.now();
  const tick = () => {
    requestAnimationFrame(tick);
    if (document.hidden) return;
    const t = (performance.now() - start) / 1000;
    for (const st of scenes) {
      if (!st.visible) continue;
      st.uniforms.uIn.value += (st.target - st.uniforms.uIn.value) * 0.03;
      st.mouse[0] += (st.mt[0] - st.mouse[0]) * 0.06; st.mouse[1] += (st.mt[1] - st.mouse[1]) * 0.06;
      st.uniforms.uMouse.value.set(st.mouse[0], st.mouse[1]);
      st.uniforms.uTime.value = t;
      st.renderer.render(st.scene, st.camera);
    }
  };
  tick();

  // ---- notes: a JSON asset if we can read it, else the markdown body ----
  const KEYS = [['graph', /graph|lane|history|commit list/i], ['fetch', /fetch|remote|pull|sync|poll/i], ['search', /search|find|filter|jump/i], ['diff', /diff|stage|hunk|patch|change/i], ['speed', /fast|speed|perf|quick|instant|ms\b/i], ['pr', /pull request|\bpr\b|review|check/i], ['sidebar', /branch|sidebar|folder|worktree|tag/i], ['accent', /accent|theme|colou?r|dark|light/i]];
  const guessScene = (f) => f.scene && SCENES[f.scene] ? f.scene : (KEYS.find(([, re]) => re.test(f.title + ' ' + (f.body || ''))) || ['eclipse'])[0];
  const splitItem = (s) => { const m = s.match(/^\*{0,2}(.+?)\*{0,2}\s*(?::|—|–|-)\s+(.+)$/); return m ? { title: m[1].trim(), body: m[2].trim() } : { title: s.trim(), body: '' }; };
  const parseBody = (md) => {
    const out = { features: [], fixes: [], improvements: [], summary: '' };
    let cur = null;
    for (const raw of (md || '').split('\n')) {
      const l = raw.trim();
      const h = l.match(/^#{1,3}\s*(.+)$/);
      if (h) { const t = h[1].toLowerCase(); cur = /feature|new/.test(t) ? 'features' : /fix|bug/.test(t) ? 'fixes' : /improv|polish|change/.test(t) ? 'improvements' : null; continue; }
      if (/^[-*]\s+/.test(l) && cur) out[cur].push(splitItem(l.replace(/^[-*]\s+/, '')));
      else if (l && !cur && !out.summary && !/^#/.test(l)) out.summary = l;
    }
    return out;
  };
  const readNotes = async (rel) => {
    const asset = (rel.assets || []).find((a) => a.name === 'release-notes.json');
    if (asset) {
      try {
        const r = await fetch(asset.url, { headers: { Accept: 'application/octet-stream' } });
        if (r.ok) { const j = await r.json(); return { features: j.features || [], fixes: j.fixes || [], improvements: j.improvements || [], summary: j.summary || '', title: j.title || '' }; }
      } catch (e) { /* CORS or network: fall back to the body */ }
    }
    return parseBody(rel.body);
  };

  const fmtDate = (s) => new Date(s).toLocaleDateString('en-GB', { day: 'numeric', month: 'long', year: 'numeric' });
  const fmtSize = (b) => `${(b / 1048576).toFixed(1)} MB`;
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };

  const render = (rel, notes, { sample: isSample = false } = {}) => {
    const node = tpl.content.firstElementChild.cloneNode(true);
    const feats = notes.features;
    const star = feats.find((f) => f.highlight) || feats[0];
    const sceneName = star ? guessScene(star) : 'fixes';
    node.dataset.scene = sceneName;
    if (isSample) node.classList.add('rel--sample');
    if (rel.prerelease) node.classList.add('is-pre');
    node.querySelector('.rel__tag').textContent = rel.tag_name;
    const time = node.querySelector('.rel__date'); time.textContent = fmtDate(rel.published_at || rel.created_at); time.dateTime = rel.published_at || rel.created_at;
    node.querySelector('.rel__title').textContent = notes.title || rel.name || rel.tag_name;
    node.querySelector('.rel__lead').textContent = notes.summary || (star ? `This one is about ${star.title.toLowerCase()}.` : feats.length ? '' : 'No new feature this time: fixes and polish, which is what makes a release feel solid.');
    node.querySelector('.rel__scene-label').textContent = star ? star.title : 'Fixes and polish';
    const groups = node.querySelector('.rel__groups');
    for (const [key, label] of [['features', 'Features'], ['improvements', 'Improvements'], ['fixes', 'Fixes']]) {
      if (!notes[key].length) continue;
      const g = el('section', `rel__group rel__group--${key}`); g.append(el('h3', null, label));
      const ul = el('ul');
      for (const it of notes[key]) { const li = el('li'), t = el('p'); t.append(el('b', null, it.title)); if (it.body) { t.append(' '); t.append(el('span', null, it.body)); } li.append(t); ul.append(li); }
      g.append(ul); groups.append(g);
    }
    const assets = node.querySelector('.rel__assets');
    const dmg = (rel.assets || []).find((a) => /\.dmg$/i.test(a.name)), exe = (rel.assets || []).find((a) => /\.(exe|msi)$/i.test(a.name));
    for (const [a, os] of [[dmg, 'macOS'], [exe, 'Windows']]) {
      if (!a) continue;
      const b = el('a', 'button' + (os === 'Windows' ? ' button--ghost' : ''), `Download for ${os} `); b.href = a.browser_download_url; b.append(el('small', null, fmtSize(a.size))); assets.append(b);
    }
    const gh = el('a', 'button button--ghost', 'Notes on GitHub'); gh.href = rel.html_url; assets.append(gh);
    list.append(node);
    mountScene(node.querySelector('.rel__canvas'), sceneName);
    return node;
  };

  const SAMPLE = {
    tag_name: 'v0.1.0', name: 'Mugetsu 0.1', published_at: '2026-10-15T00:00:00Z', html_url: 'https://github.com/ziracmo/mugetsu/releases', assets: [],
    notes: { summary: 'The first build you can download: the graph, the sidebar and the diff, drawn by the GPU.', title: 'Mugetsu 0.1',
      features: [{ title: 'A commit graph that scrolls like a list', body: 'Lanes laid out once, rows painted only where you look.', highlight: true }, { title: 'Branches in folders', body: 'feat/ and fix/ become folders; worktrees open in a tab.' }, { title: 'Diffs down to the character', body: 'Syntax highlighting and intra-line changes.' }],
      improvements: [], fixes: [] },
  };
  const SAMPLE_FIX = { tag_name: 'v0.1.1', name: 'Mugetsu 0.1.1', published_at: '2026-10-15T00:00:00Z', html_url: 'https://github.com/ziracmo/mugetsu/releases', assets: [],
    notes: { summary: '', title: 'Mugetsu 0.1.1', features: [], improvements: [{ title: 'Faster first paint', body: 'The window opens before the repository is read.' }], fixes: [{ title: 'Stash pop on a dirty tree', body: 'No longer drops the stash on conflict.' }, { title: 'Avatars behind a proxy', body: 'Gravatar lookups respect the system proxy.' }] } };

  const empty = (msg) => { const p = el('p', 'rel__empty', msg); list.append(p); };

  const load = async () => {
    let rels = [];
    try {
      const r = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=30`, { headers: { Accept: 'application/vnd.github+json' } });
      if (!r.ok) throw new Error(r.status);
      rels = (await r.json()).filter((x) => !x.draft);
    } catch (e) {
      count.textContent = 'GitHub could not be reached, so the two below are samples.';
      empty('The release list could not be loaded from GitHub. Here is what a release looks like, with sample notes.');
      render(SAMPLE, SAMPLE.notes, { sample: true }); render(SAMPLE_FIX, SAMPLE_FIX.notes, { sample: true });
      return;
    }
    if (!rels.length) {
      count.textContent = 'none published yet, so the two below are samples.';
      empty('The first release is on its way. Here is what one looks like, with sample notes.');
      render(SAMPLE, SAMPLE.notes, { sample: true }); render(SAMPLE_FIX, SAMPLE_FIX.notes, { sample: true });
      return;
    }
    count.textContent = `${rels.length} release${rels.length > 1 ? 's' : ''} so far, the latest is ${rels[0].tag_name}.`;
    for (const rel of rels) render(rel, await readNotes(rel));
  };

  // ---- reveals and nav, same as the home page ----
  const revealAll = () => document.querySelectorAll('[data-reveal], [data-lines]').forEach((e) => e.classList.add('is-in'));
  if (M && !reduced && 'IntersectionObserver' in window) {
    const seen = new WeakSet();
    const watch = () => document.querySelectorAll('[data-reveal]').forEach((e) => { if (seen.has(e)) return; seen.add(e); M.inView(e, () => e.classList.add('is-in'), { margin: '0px 0px -10% 0px' }); });
    watch(); new MutationObserver(watch).observe(list, { childList: true });
    addEventListener('load', () => setTimeout(revealAll, 4000));
  } else revealAll();
  const nav = document.getElementById('nav');
  const onNav = () => nav.classList.toggle('is-scrolled', scrollY > 40);
  addEventListener('scroll', onNav, { passive: true }); onNav();
  load();
})();
