// The eclipse behind the hero: a full-screen Three.js shader. A black disc, a crimson corona of
// drifting noise, a thin bright rim, faint stars. Mouse moves it a little; scrolling lifts and dims it.
// If WebGL is missing or reduced motion is on, the CSS eclipse stays and this does nothing.
(() => {
  const canvas = document.getElementById('eclipse');
  if (!canvas || typeof THREE === 'undefined') return;
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
    uScroll: { value: 0 },
    uColor: { value: new THREE.Vector3(...accent()) },
  };

  const frag = `
    precision highp float;
    uniform vec2 uRes; uniform float uTime; uniform vec2 uMouse; uniform float uScroll; uniform vec3 uColor;
    float hash(vec2 p) { p = fract(p * vec2(123.34, 456.21)); p += dot(p, p + 45.32); return fract(p.x * p.y); }
    float noise(vec2 p) {
      vec2 i = floor(p), f = fract(p); f = f * f * (3.0 - 2.0 * f);
      return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
    }
    float fbm(vec2 p) { float v = 0.0, a = 0.5; for (int i = 0; i < 5; i++) { v += a * noise(p); p = p * 2.03 + 11.7; a *= 0.5; } return v; }
    void main() {
      vec2 uv = (gl_FragCoord.xy - 0.5 * uRes) / uRes.y;
      // the sun barely moves with the pointer; the moon, closer to us, moves more
      vec2 cs = vec2(0.0, 0.19 + uScroll * 0.35) + uMouse * 0.006;
      vec2 cm = cs + vec2(-0.028, 0.012) + uMouse * 0.028 + vec2(sin(uTime * 0.11), cos(uTime * 0.09)) * 0.004;
      vec2 ds = uv - cs; vec2 dm = uv - cm;
      float rs = length(ds); float rm = length(dm);
      float ang = atan(ds.y, ds.x);
      float R = 0.21;   // the sun
      float Rm = 0.215; // the moon, a hair larger
      // stars
      vec2 sp = uv * 420.0;
      float star = step(0.9985, hash(floor(sp))) * (0.5 + 0.5 * sin(uTime * 0.8 + hash(floor(sp) + 3.0) * 6.28));
      vec3 col = vec3(star) * 0.45;
      // corona around the sun: noise in polar space, drifting outward
      float n = fbm(vec2(ang * 2.2 + uTime * 0.05, rs * 5.0 - uTime * 0.12));
      float n2 = fbm(vec2(ang * 7.0 - uTime * 0.03, rs * 14.0 - uTime * 0.25));
      float flare = pow(max(0.0, 1.0 - (rs - R) / 0.3), 3.0) * (0.55 + 0.9 * n) * (0.7 + 0.6 * n2);
      // the light leans to the side the moon has not covered
      vec2 lean = normalize(cs - cm + vec2(0.0001));
      float side = 0.55 + 0.45 * dot(normalize(ds + vec2(0.0001)), lean);
      flare *= side;
      // the sun's photosphere: a bright disc with a soft limb
      float sun = smoothstep(R + 0.002, R - 0.004, rs);
      vec3 sunCol = mix(uColor, vec3(1.0), 0.78) * (0.9 + 0.1 * n2);
      // beam from the top
      float beam = exp(-abs(uv.x - cs.x) * 120.0) * smoothstep(cs.y + R, cs.y + R + 0.9, uv.y) * 0.3;
      // god rays: thin streaks of light radiating from the uncovered side, slowly turning
      float rays = pow(noise(vec2(ang * 9.0 + uTime * 0.04, 1.7)), 3.0) * pow(max(0.0, 1.0 - (rs - R) / 0.9), 1.6) * step(R, rs);
      rays *= 0.55 + 0.45 * dot(normalize(ds + vec2(0.0001)), lean);
      // an anamorphic streak across the crescent
      float streak = exp(-abs(uv.y - cs.y - 0.02) * 90.0) * exp(-abs(uv.x - cs.x) * 2.2) * 0.35 * (0.8 + 0.2 * sin(uTime * 0.7));
      // drifting dust, caught in the light
      vec2 dp = uv * 60.0 + vec2(uTime * 0.25, uTime * 0.6);
      float dust = step(0.985, hash(floor(dp))) * smoothstep(0.5, 0.0, length(fract(dp) - 0.5)) * (0.4 + 0.6 * sin(uTime * 1.3 + hash(floor(dp) + 9.0) * 6.28));
      dust *= smoothstep(1.1, 0.2, rs) * 0.8;
      col += uColor * flare * 1.3 + uColor * beam + mix(uColor, vec3(1.0), 0.5) * rays * 0.3 + mix(uColor, vec3(1.0), 0.7) * streak * 0.0 + mix(uColor, vec3(1.0), 0.6) * dust;
      col = mix(col, sunCol, sun);
      col += vec3(1.0) * smoothstep(0.012, 0.0, abs(rs - R)) * 0.35;
      // the moon covers most of the sun and leaves a crescent
      float moon = smoothstep(Rm + 0.0015, Rm - 0.0015, rm);
      float moonTex = fbm(dm * 9.0 + 4.0) * 0.5 + fbm(dm * 26.0) * 0.25;
      float earthshine = smoothstep(Rm, Rm * 0.35, rm) * 0.015;
      vec3 moonCol = vec3(0.016, 0.009, 0.012) + vec3(moonTex * 0.025) + uColor * earthshine
                   + uColor * 0.16 * smoothstep(Rm - 0.05, Rm, rm) * (0.5 + 0.5 * dot(normalize(dm + vec2(0.0001)), lean));
      col = mix(col, moonCol, moon);
      // vignette and scroll dim
      col *= 1.0 - smoothstep(0.55, 1.1, length(uv)) * 0.6;
      col *= 1.0 - uScroll * 0.7;
      gl_FragColor = vec4(col, 1.0);
    }`;
  const vert = 'void main(){ gl_Position = vec4(position, 1.0); }';

  const scene = new THREE.Scene();
  const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const mat = new THREE.ShaderMaterial({ uniforms, vertexShader: vert, fragmentShader: frag });
  scene.add(new THREE.Mesh(new THREE.PlaneGeometry(2, 2), mat));

  const hero = canvas.parentElement;
  const resize = () => {
    const w = hero.clientWidth, hh = hero.clientHeight;
    renderer.setSize(w, hh, false);
    uniforms.uRes.value.set(renderer.domElement.width, renderer.domElement.height);
  };
  resize();
  addEventListener('resize', resize);
  document.documentElement.classList.add('gl');

  let target = [0, 0], mouse = [0, 0];
  addEventListener('pointermove', (e) => { target = [(e.clientX / innerWidth - 0.5) * 2, -(e.clientY / innerHeight - 0.5) * 2]; }, { passive: true });
  const onScroll = () => { uniforms.uScroll.value = Math.min(1, scrollY / Math.max(1, hero.clientHeight)); };
  addEventListener('scroll', onScroll, { passive: true });
  new MutationObserver(() => uniforms.uColor.value.set(...accent())).observe(document.documentElement, { attributes: true, attributeFilter: ['style'] });

  let visible = true;
  new IntersectionObserver(([e]) => { visible = e.isIntersecting; }).observe(hero);
  const start = performance.now();
  const tick = () => {
    requestAnimationFrame(tick);
    if (!visible || document.hidden) return;
    mouse[0] += (target[0] - mouse[0]) * 0.04; mouse[1] += (target[1] - mouse[1]) * 0.04;
    uniforms.uMouse.value.set(mouse[0], mouse[1]);
    uniforms.uTime.value = (performance.now() - start) / 1000;
    renderer.render(scene, camera);
  };
  tick();
})();
