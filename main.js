// Mugetsu site: accent picker, history-time slider, shortcut tabs, install dialog, reveals.
(() => {
  const root = document.documentElement;
  const M = window.Motion;

  // ---- accent colour (the same eight as the app's Settings window) ----
  const swatches = [...document.querySelectorAll('.sw')];
  const name = document.getElementById('accent-name');
  const lightness = (hex) => { const n = parseInt(hex.slice(1), 16); return (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255; };
  const preview = document.querySelector('.ui-preview');
  const burst = document.getElementById('burst');
  const flashPreview = () => {
    if (!preview) return;
    [...preview.children].forEach((r, i) => r.style.setProperty('--r', i));
    preview.classList.remove('is-flash'); void preview.offsetWidth; preview.classList.add('is-flash');
    if (burst && window.Motion) {
      burst.innerHTML = '';
      for (let i = 0; i < 14; i++) {
        const p = document.createElement('i'); burst.appendChild(p);
        const a = (i / 14) * Math.PI * 2, d = 18 + Math.random() * 22;
        window.Motion.animate(p, { opacity: [1, 0], transform: [`translate(-50%, -50%) scale(1)`, `translate(calc(-50% + ${Math.cos(a) * d}px), calc(-50% + ${Math.sin(a) * d}px)) scale(0.2)`] }, { duration: 0.7 + Math.random() * 0.3, easing: [0.22, 1, 0.36, 1] });
      }
    }
  };
  const setAccent = (btn, save) => {
    swatches.forEach((s) => s.setAttribute('aria-checked', String(s === btn)));
    root.style.setProperty('--accent', btn.dataset.c);
    root.style.setProperty('--on-accent', lightness(btn.dataset.c) > 0.55 ? '#07060a' : '#ffffff');
    name.textContent = btn.dataset.n;
    flashPreview();
    if (save) { try { localStorage.setItem('mugetsu-accent', btn.dataset.c); } catch (e) {} }
  };
  let saved = null;
  try { saved = localStorage.getItem('mugetsu-accent'); } catch (e) {}
  const initial = swatches.find((s) => s.dataset.c === saved);
  if (initial) setAccent(initial, false);
  swatches.forEach((s, i) => {
    s.addEventListener('click', () => setAccent(s, true));
    s.addEventListener('keydown', (e) => {
      const d = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
      if (!d) return;
      e.preventDefault();
      const next = swatches[(i + d + swatches.length) % swatches.length];
      next.focus(); setAccent(next, true);
    });
  });
  const focusable = () => { const on = swatches.find((s) => s.getAttribute('aria-checked') === 'true') || swatches[0]; swatches.forEach((s) => s.tabIndex = s === on ? 0 : -1); };
  focusable();
  new MutationObserver(focusable)
    .observe(document.querySelector('.ui-accent__row'), { attributes: true, subtree: true, attributeFilter: ['aria-checked'] });

  // ---- time to open history, scaled from the git.git measurement ----
  // git.git: 85,593 commits, history 0.66 s (linear), graph layout 13 ms (n log n), refs 40 ms.
  const REF_N = 85593, HIST = 0.66, LAYOUT = 0.013, REFS = 0.04;
  const slider = document.getElementById('commits');
  const out = document.getElementById('commits-out');
  const num = document.getElementById('speed-num');
  const unit = document.getElementById('speed-unit');
  const fmt = new Intl.NumberFormat('en-US');
  const commitsFor = (v) => {
    if (Number(v) === 644) return REF_N;
    const n = 1000 * Math.pow(1000, v / 1000);
    const mag = Math.pow(10, Math.floor(Math.log10(n)) - 1);
    return Math.round(n / mag) * mag;
  };
  const timeFor = (n) => HIST * n / REF_N + LAYOUT * (n * Math.log2(n)) / (REF_N * Math.log2(REF_N)) + REFS;
  const showTime = (t) => {
    if (t < 1) { num.textContent = String(Math.round(t * 1000)); unit.textContent = 'ms'; }
    else { num.textContent = t < 10 ? t.toFixed(2) : t.toFixed(1); unit.textContent = 's'; }
  };
  const lanes = [...document.querySelectorAll('.lane')];
  let raceRun = null;
  const race = () => {
    const n = commitsFor(slider.value), t = timeFor(n);
    if (raceRun) raceRun.forEach((a) => a.stop && a.stop());
    raceRun = [];
    lanes.forEach((l) => l.classList.remove('is-done'));
    const me = lanes[0];
    // Mugetsu's bar crosses the track in the real time, shown on a 1 s = 1.6 s clock; the others keep crawling
    const clock = Math.min(2.6, Math.max(0.9, t * 1.6));
    if (!M || matchMedia('(prefers-reduced-motion: reduce)').matches) { lanes.forEach((l, i) => { l.querySelector('.lane__bar').style.width = i ? '38%' : 'calc(100% - 12px)'; }); showTime(t); me.classList.add('is-done'); return; }
    raceRun.push(M.animate(me.querySelector('.lane__bar'), { width: ['0%', 'calc(100% - 12px)'] }, { duration: clock, easing: [0.33, 1, 0.68, 1] }));
    raceRun.push(M.animate((p) => showTime(t * p), { duration: clock, easing: [0.33, 1, 0.68, 1] }));
    setTimeout(() => me.classList.add('is-done'), clock * 1000);
    lanes.slice(1).forEach((l, i) => {
      raceRun.push(M.animate(l.querySelector('.lane__bar'), { width: ['0%', `${22 + i * 7}%`] }, { duration: clock * 2.2, easing: 'linear' }));
    });
  };
  const update = () => {
    const n = commitsFor(slider.value);
    out.textContent = fmt.format(n);
    slider.setAttribute('aria-valuetext', fmt.format(n) + ' commits');
    showTime(timeFor(n));
  };
  slider.addEventListener('input', update);
  slider.addEventListener('change', race);
  document.getElementById('replay').addEventListener('click', race);
  update();

  // ---- shortcut tabs ----
  const tabs = [...document.querySelectorAll('.keys__tabs [role="tab"]')];
  const select = (tab) => {
    tabs.forEach((t) => {
      const on = t === tab;
      t.setAttribute('aria-selected', String(on));
      t.tabIndex = on ? 0 : -1;
      const panel = document.getElementById(t.getAttribute('aria-controls'));
      panel.hidden = !on;
      if (on) [...panel.children].forEach((li, i) => li.style.setProperty('--r', i));
    });
  };
  tabs.forEach((t, i) => {
    t.addEventListener('click', () => select(t));
    t.addEventListener('keydown', (e) => {
      const d = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
      if (!d) return;
      e.preventDefault();
      const next = tabs[(i + d + tabs.length) % tabs.length];
      next.focus(); select(next);
    });
  });

  // ---- install dialog ----
  const form = document.getElementById('install-form');
  const dir = document.getElementById('dir');
  const cmd = document.getElementById('cmd-text');
  const copyBtn = document.getElementById('copy');
  const modes = [...form.querySelectorAll('[data-mode]')];
  let mode = 'app';
  const render = () => {
    const d = dir.value.trim() || '~/code/mugetsu';
    const q = /[\s'"]/.test(d) && !d.startsWith('~') ? `"${d}"` : d;
    const last = mode === 'app' ? 'scripts/bundle-macos.sh' : 'cargo run --release -p mugetsu-app -- ~/code/my-repo';
    cmd.textContent = `git clone https://github.com/ziracmo/mugetsu ${q}\ncd ${q}\n${last}`;
  };
  modes.forEach((b) => b.addEventListener('click', () => {
    mode = b.dataset.mode;
    modes.forEach((m) => m.setAttribute('aria-checked', String(m === b)));
    render();
  }));
  dir.addEventListener('input', render);
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    try { await navigator.clipboard.writeText(cmd.textContent); copyBtn.textContent = 'Copied'; }
    catch (err) { getSelection().selectAllChildren(cmd); copyBtn.textContent = 'Press ⌘C'; }
    setTimeout(() => { copyBtn.textContent = 'Copy command'; }, 1800);
  });

  // ---- competitor stack ----
  const CLIENTS = {
    mugetsu: {
      layers: [['Rust', 'your click, handled in native code', 'is-native'], ['GPUI', 'the UI framework, draws through Metal', 'is-native']],
      facts: ['Native Rust, no web view', 'The GPU, only visible rows', 'gix in process, git CLI for the rest', 'Free', 'Open source, MIT'],
    },
    fork: {
      layers: [['Native Cocoa', 'the macOS toolkit', 'is-native'], ['Git CLI', 'spawned for every operation', '']],
      facts: ['Native Cocoa on macOS, .NET on Windows', 'The CPU, in a native view', 'The git command line', 'Paid licence after an evaluation period', 'Closed'],
    },
    kraken: {
      layers: [['JavaScript', 'the app logic', 'is-heavy'], ['DOM and CSS', 'every row is an element', 'is-heavy'], ['Chromium', 'a bundled browser', 'is-heavy'], ['Node.js', 'the host process', 'is-heavy']],
      facts: ['Electron', 'The browser engine', 'Its own JavaScript implementation', 'Free tier, paid plans for teams and features', 'Closed'],
    },
    ghd: {
      layers: [['React', 'the app logic', 'is-heavy'], ['DOM and CSS', 'every row is an element', 'is-heavy'], ['Chromium', 'a bundled browser', 'is-heavy'], ['Node.js', 'the host process', 'is-heavy']],
      facts: ['Electron', 'The browser engine', 'The git command line (dugite)', 'Free', 'Open source, MIT'],
    },
  };
  const pick = [...document.querySelectorAll('[data-client]')];
  const layersEl = document.getElementById('stack-layers');
  const factEls = ['f-runtime', 'f-graph', 'f-engine', 'f-price', 'f-source'].map((id) => document.getElementById(id));
  const showClient = (key) => {
    pick.forEach((b) => { const on = b.dataset.client === key; b.setAttribute('aria-selected', String(on)); b.tabIndex = on ? 0 : -1; });
    const c = CLIENTS[key];
    layersEl.innerHTML = c.layers.map(([n, d, cls]) => `<li class="${cls}"><span>${n}</span><small>${d}</small></li>`).join('');
    c.facts.forEach((f, i) => { factEls[i].textContent = f; });
    if (M && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
      M.animate(layersEl.children, { opacity: [0, 1], transform: ['translateY(-14px) scale(0.98)', 'none'] }, { delay: M.stagger(0.09), duration: 0.5, easing: [0.22, 1, 0.36, 1] });
      M.animate(factEls, { opacity: [0, 1] }, { delay: M.stagger(0.06), duration: 0.4 });
    }
  };
  pick.forEach((b, i) => {
    b.addEventListener('click', () => showClient(b.dataset.client));
    b.addEventListener('keydown', (e) => {
      const d = { ArrowRight: 1, ArrowLeft: -1 }[e.key]; if (!d) return; e.preventDefault();
      const n = pick[(i + d + pick.length) % pick.length]; n.focus(); showClient(n.dataset.client);
    });
  });
  showClient('mugetsu');

  // ---- downloads: look at the latest release for a .dmg and a .exe ----
  const dlCards = { mac: { el: document.getElementById('dl-mac'), ext: '.dmg', soon: 'Signed .dmg coming with the first release' }, win: { el: document.getElementById('dl-win'), ext: '.exe', soon: 'Windows build in progress' } };
  const setSoon = () => Object.values(dlCards).forEach(({ el, soon }) => { el.classList.add('is-soon'); el.querySelector('.dl__state').textContent = soon; });
  fetch('https://api.github.com/repos/ziracmo/mugetsu/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then((rel) => {
      Object.values(dlCards).forEach(({ el, ext, soon }) => {
        const asset = (rel.assets || []).find((a) => a.name.toLowerCase().endsWith(ext));
        const state = el.querySelector('.dl__state');
        if (asset) {
          el.href = asset.browser_download_url;
          el.classList.add('is-ready');
          el.querySelector('.dl__file').textContent = asset.name;
          state.textContent = `${rel.tag_name} · ${(asset.size / 1048576).toFixed(1)} MB`;
        } else { el.classList.add('is-soon'); state.textContent = soon; }
      });
    })
    .catch(setSoon);

  // ---- tiles: the glow follows the pointer ----
  document.querySelectorAll('[data-tile]').forEach((t) => {
    t.addEventListener('pointermove', (e) => {
      const r = t.getBoundingClientRect();
      t.style.setProperty('--mx', `${((e.clientX - r.left) / r.width) * 100}%`);
      t.style.setProperty('--my', `${((e.clientY - r.top) / r.height) * 100}%`);
    }, { passive: true });
  });

  // ---- reveals ----
  const targets = document.querySelectorAll('[data-reveal]');
    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (M && !reduced && 'IntersectionObserver' in window) {
    // sections: the CSS reveal, triggered by Motion's inView
    targets.forEach((el) => M.inView(el, () => { el.classList.add('is-in'); }, { margin: '0px 0px -10% 0px' }));
    document.querySelectorAll('[data-lines]').forEach((el) => M.inView(el, () => { el.classList.add('is-in'); }, { margin: '0px 0px -10% 0px' }));
    document.querySelectorAll('[data-rows]').forEach((el) => { [...el.children].forEach((li, i) => li.style.setProperty('--r', i)); M.inView(el, () => { el.classList.add('is-in'); }, { margin: '0px 0px -10% 0px' }); });
    M.inView('#race', () => { race(); }, { margin: '0px 0px -25% 0px' });
    // the footer: letters rise, the moon slides over the sun with the scroll
    const mark = document.getElementById('foot-mark');
    if (mark) {
      M.inView(mark, () => { mark.classList.add('is-in'); }, { margin: '0px 0px -15% 0px' });
    }
    // the hero title leans toward the pointer
    const copy = document.getElementById('hero-copy');
    if (copy && matchMedia('(hover: hover)').matches) {
      const word = copy.querySelector('.hero__title');
      let tx = 0, ty = 0, cx = 0, cy = 0, raf = 0;
      const step = () => { cx += (tx - cx) * 0.08; cy += (ty - cy) * 0.08; word.style.transform = `translate(${cx}px, ${cy}px)`; if (Math.abs(tx - cx) > 0.1 || Math.abs(ty - cy) > 0.1) raf = requestAnimationFrame(step); else raf = 0; };
      addEventListener('pointermove', (e) => { tx = (e.clientX / innerWidth - 0.5) * 18; ty = (e.clientY / innerHeight - 0.5) * 10; if (!raf) raf = requestAnimationFrame(step); }, { passive: true });
    }
    // the band drifts with the scroll
    const band = document.querySelector('.band img');
    if (band) M.scroll(M.animate(band, { transform: ['translateY(-8%)', 'translateY(2%)'] }, { easing: 'linear' }), { target: document.querySelector('.band'), offset: ['start end', 'end start'] });
    // safety net: anything still hidden after load is shown
    addEventListener('load', () => setTimeout(() => { targets.forEach((el) => el.classList.add('is-in')); }, 3000));
  } else {
    targets.forEach((el) => el.classList.add('is-in'));
    document.querySelectorAll('[data-lines], [data-rows], #foot-mark').forEach((el) => el.classList.add('is-in'));
    race();
  }
  // nav: a blurred bar once the hero is left behind
  const nav = document.getElementById('nav');
  const onNav = () => nav.classList.toggle('is-scrolled', scrollY > 40);
  addEventListener('scroll', onNav, { passive: true }); onNav();
})();
