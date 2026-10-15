// The footer: a starry night, a thin eclipse over the wordmark, and a dark liquid below the horizon
// that reflects it in slow ripples. Same recipe as the hero shader, quieter.
(() => {
  const foot = document.getElementById('foot-mark');
  const canvas = document.getElementById('night');
  if (!foot || !canvas || typeof THREE === 'undefined') return;
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;

  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ canvas, antialias: false, alpha: true, powerPreference: 'high-performance' });
  } catch (e) { return; }
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));

  const hexToRgb = (hex) => {
    const n = parseInt(hex.replace('#', ''), 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
  };
  const accent = () => hexToRgb(getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#b5122f');

  const uniforms = {
    uRes: { value: new THREE.Vector2(1, 1) },
    uTime: { value: 0 },
    uMouse: { value: new THREE.Vector2(0, 0) },
    uIn: { value: 0 },
    uHorizon: { value: 0.3 },
    uColor: { value: new THREE.Vector3(...accent()) },
  };

  const frag = `
    precision highp float;
    uniform vec2 uRes; uniform float uTime; uniform vec2 uMouse; uniform float uIn; uniform float uHorizon; uniform vec3 uColor;
    float hash(vec2 p) { p = fract(p * vec2(123.34, 456.21)); p += dot(p, p + 45.32); return fract(p.x * p.y); }
    float noise(vec2 p) {
      vec2 i = floor(p), f = fract(p); f = f * f * (3.0 - 2.0 * f);
      return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
    }
    float fbm(vec2 p) { float v = 0.0, a = 0.5; for (int i = 0; i < 5; i++) { v += a * noise(p); p = p * 2.03 + 11.7; a *= 0.5; } return v; }
    vec3 sky(vec2 uv, vec2 cs, vec2 cm, float R, float Rm) {
      vec2 ds = uv - cs, dm = uv - cm; float rs = length(ds), rm = length(dm); float ang = atan(ds.y, ds.x);
      // stars, two layers, slow twinkle
      vec2 sp = uv * 300.0;
      float star = step(0.996, hash(floor(sp))) * (0.5 + 0.5 * sin(uTime * 0.9 + hash(floor(sp) + 3.0) * 6.28));
      vec2 sp2 = uv * 140.0 + 7.0;
      float star2 = step(0.995, hash(floor(sp2))) * (0.6 + 0.4 * sin(uTime * 0.5 + hash(floor(sp2) + 5.0) * 6.28)) * smoothstep(0.5, 0.0, length(fract(sp2) - 0.5));
      vec3 col = vec3(star) * 0.5 + vec3(1.0, 0.95, 0.95) * star2 * 0.9;
      // a faint milky band
      col += vec3(0.07, 0.05, 0.07) * fbm(uv * 3.0 + vec2(0.0, uTime * 0.01)) * smoothstep(0.6, 0.0, abs(uv.y - 0.15 + uv.x * 0.25));
      // the corona
      float n = fbm(vec2(ang * 2.2 + uTime * 0.05, rs * 5.0 - uTime * 0.12));
      float flare = pow(max(0.0, 1.0 - (rs - R) / 0.26), 3.0) * (0.5 + 0.9 * n);
      vec2 lean = normalize(cs - cm + vec2(0.0001));
      flare *= 0.5 + 0.5 * dot(normalize(ds + vec2(0.0001)), lean);
      float rays = pow(noise(vec2(ang * 9.0 + uTime * 0.04, 1.7)), 3.0) * pow(max(0.0, 1.0 - (rs - R) / 0.7), 1.6) * step(R, rs);
      col += uColor * flare * 1.1 + mix(uColor, vec3(1.0), 0.5) * rays * 0.2;
      float sun = smoothstep(R + 0.003, R - 0.008, rs);
      col = mix(col, mix(uColor, vec3(1.0), 0.8), sun);
      col += vec3(1.0) * smoothstep(0.01, 0.0, abs(rs - R)) * 0.3;
      float moon = smoothstep(Rm + 0.002, Rm - 0.003, rm);
      float moonTex = fbm(dm * 9.0 + 4.0) * 0.5 + fbm(dm * 26.0) * 0.25;
      vec3 moonCol = vec3(0.012, 0.007, 0.01) + vec3(moonTex * 0.02) + uColor * 0.12 * smoothstep(Rm - 0.05, Rm, rm) * (0.5 + 0.5 * dot(normalize(dm + vec2(0.0001)), lean));
      return mix(col, moonCol, moon);
    }
    void main() {
      vec2 uv = (gl_FragCoord.xy - 0.5 * uRes) / uRes.y;
      float h = uHorizon;
      vec2 cs = vec2(0.0, h + 0.34) + uMouse * 0.004;
      vec2 cm = cs + vec2(-0.02, 0.01) + uMouse * 0.018 + vec2(sin(uTime * 0.11), cos(uTime * 0.09)) * 0.003;
      float R = 0.1, Rm = 0.103;
      vec3 col;
      if (uv.y > h) {
        col = sky(uv, cs, cm, R, Rm);
        // the glow pools just above the horizon
        col += uColor * 0.25 * exp(-(uv.y - h) * 9.0) * exp(-abs(uv.x) * 1.2);
      } else {
        // the liquid: a dark surface, rippling, that mirrors the sky with a slow distortion
        float depth = h - uv.y;
        vec2 rp = vec2(uv.x * 4.0, depth * 18.0);
        float w = fbm(rp + vec2(uTime * 0.08, -uTime * 0.25)) - 0.5;
        float w2 = fbm(rp * 2.3 + vec2(-uTime * 0.12, uTime * 0.3)) - 0.5;
        vec2 muv = vec2(uv.x + w * 0.05 * depth * 4.0 + w2 * 0.015, h + depth + w * 0.06 + w2 * 0.02);
        vec3 refl = sky(muv, cs, cm, R, Rm);
        float fres = pow(1.0 - clamp(depth * 1.6, 0.0, 1.0), 1.2);
        vec3 deep = mix(vec3(0.04, 0.012, 0.025), uColor * 0.3, smoothstep(0.0, 0.4, depth) * 0.7);
        col = mix(deep, refl * 1.3, fres);
        // crests catch the light
        float crest = smoothstep(0.08, 0.22, w + w2 * 0.6) * exp(-depth * 4.0);
        col += mix(uColor, vec3(1.0), 0.6) * crest * 0.6 * exp(-abs(uv.x) * 1.2);
        // the horizon line itself
        col += mix(uColor, vec3(1.0), 0.7) * smoothstep(0.004, 0.0, depth) * 0.5 * exp(-abs(uv.x) * 1.0);
      }
      col *= 1.0 - smoothstep(0.7, 1.3, length(vec2(uv.x * 0.8, uv.y - h))) * 0.6;
      // the liquid fades into the page before the links
      col *= smoothstep(-0.5, -0.42, uv.y);
      col *= uIn;
      gl_FragColor = vec4(col, 1.0);
    }`;
  const vert = 'void main(){ gl_Position = vec4(position, 1.0); }';

  const scene = new THREE.Scene();
  const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const mat = new THREE.ShaderMaterial({ uniforms, vertexShader: vert, fragmentShader: frag });
  scene.add(new THREE.Mesh(new THREE.PlaneGeometry(2, 2), mat));

  const mark = foot;
  const resize = () => {
    const w = foot.clientWidth, hh = foot.clientHeight;
    renderer.setSize(w, hh, false);
    uniforms.uRes.value.set(renderer.domElement.width, renderer.domElement.height);
    // the horizon sits at the baseline of the wordmark
    const word = mark.querySelector('.foot__word');
    const y = word.getBoundingClientRect().bottom - foot.getBoundingClientRect().top;
    uniforms.uHorizon.value = (hh / 2 - y) / hh;
  };
  resize();
  addEventListener('resize', resize);
  addEventListener('load', resize);
  foot.classList.add('gl');

  let target = [0, 0], mouse = [0, 0];
  addEventListener('pointermove', (e) => { target = [(e.clientX / innerWidth - 0.5) * 2, -(e.clientY / innerHeight - 0.5) * 2]; }, { passive: true });
  new MutationObserver(() => uniforms.uColor.value.set(...accent())).observe(document.documentElement, { attributes: true, attributeFilter: ['style'] });

  let visible = false;
  new IntersectionObserver(([e]) => { visible = e.isIntersecting; }).observe(foot);
  const start = performance.now();
  const tick = () => {
    requestAnimationFrame(tick);
    if (!visible || document.hidden) return;
    mouse[0] += (target[0] - mouse[0]) * 0.04; mouse[1] += (target[1] - mouse[1]) * 0.04;
    uniforms.uMouse.value.set(mouse[0], mouse[1]);
    uniforms.uTime.value = (performance.now() - start) / 1000;
    uniforms.uIn.value += (1 - uniforms.uIn.value) * 0.03;
    renderer.render(scene, camera);
  };
  tick();
})();
