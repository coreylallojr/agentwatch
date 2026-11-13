// agentwatch landing page: desktop behaviour (windows, taskbar, start menu), install helpers, demo terminal.
(() => {
  'use strict';
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];
  const REPO = 'coreylallojr/agentwatch';
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  // ---------------------------------------------------------------- windows
  const wins = $$('.win');
  const tasks = $('#tasks');
  let z = 10;
  const icon = (w) => `<svg aria-hidden="true"><use href="#${w.dataset.icon}"/></svg>`;
  const isOpen = (w) => !w.classList.contains('closed') && !w.classList.contains('minimized');

  function sync() {
    for (const w of wins) {
      const b = w._task;
      b.setAttribute('aria-pressed', String(isOpen(w)));
      b.classList.toggle('off', w.classList.contains('closed'));
    }
    $('#sm-windows').innerHTML = wins.map((w) => `<li><button role="menuitem" data-win-id="${w.id}" class="${w.classList.contains('closed') ? 'closed-win' : ''}">${icon(w)}${esc(w.dataset.win)}</button></li>`).join('');
  }

  function openWin(id, { scroll = true } = {}) {
    const w = document.getElementById(id);
    if (!w || !w.classList.contains('win')) return;
    w.classList.remove('closed', 'minimized', 'reveal');
    w.classList.add('shown');
    w.style.zIndex = ++z;
    if (scroll && !w.classList.contains('maximized')) w.scrollIntoView({ behavior: reduced ? 'auto' : 'smooth', block: 'start' });
    w.classList.remove('flash'); void w.offsetWidth; w.classList.add('flash');
    sync();
  }

  for (const w of wins) {
    const b = document.createElement('button');
    b.innerHTML = `${icon(w)}<span>${esc(w.dataset.win)}</span>`;
    b.title = w.dataset.win;
    b.addEventListener('click', () => openWin(w.id));
    tasks.appendChild(b);
    w._task = b;
    const [min, max, close] = $$('.title-bar-controls button', w);
    min?.addEventListener('click', () => { w.classList.toggle('minimized'); w.classList.remove('maximized'); sync(); });
    max?.addEventListener('click', () => { w.classList.toggle('maximized'); w.classList.remove('minimized'); w.style.zIndex = ++z; sync(); });
    close?.addEventListener('click', () => { w.classList.add('closed'); w.classList.remove('maximized'); sync(); });
    $('.title-bar', w).addEventListener('dblclick', (e) => { if (!e.target.closest('button')) max?.click(); });
    dragify(w);
  }
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') for (const w of $$('.win.maximized')) w.classList.remove('maximized'); });

  // drag windows by their title bar on big screens with a mouse
  const canDrag = matchMedia('(pointer: fine) and (min-width: 1100px)');
  function dragify(w) {
    const bar = $('.title-bar', w);
    let sx = 0, sy = 0, ox = 0, oy = 0, on = false;
    bar.addEventListener('pointerdown', (e) => {
      if (!canDrag.matches || e.button !== 0 || e.target.closest('button') || w.classList.contains('maximized')) return;
      on = true; sx = e.clientX; sy = e.clientY; ox = Number(w.dataset.x || 0); oy = Number(w.dataset.y || 0);
      w.style.zIndex = ++z; w.classList.add('dragging'); bar.setPointerCapture(e.pointerId);
    });
    bar.addEventListener('pointermove', (e) => {
      if (!on) return;
      const x = ox + e.clientX - sx, y = oy + e.clientY - sy;
      w.dataset.x = x; w.dataset.y = y; w.style.transform = `translate(${x}px, ${y}px)`;
    });
    const end = () => { on = false; w.classList.remove('dragging'); };
    bar.addEventListener('pointerup', end); bar.addEventListener('pointercancel', end);
  }

  // XP-style "open" animation as windows scroll into view
  if ('IntersectionObserver' in window && !reduced) {
    const io = new IntersectionObserver((es) => { for (const e of es) if (e.isIntersecting) { e.target.classList.remove('reveal'); e.target.classList.add('shown'); io.unobserve(e.target); } }, { threshold: 0.08 });
    for (const w of wins) if (w.id !== 'hero') { w.classList.add('reveal'); io.observe(w); }
  }

  // links that point at a window open it first
  document.addEventListener('click', (e) => {
    const a = e.target.closest('[data-open]');
    if (!a) return;
    e.preventDefault();
    openWin(a.dataset.open);
    history.replaceState(null, '', '#' + a.dataset.open);
  });
  if (location.hash.length > 1 && document.getElementById(location.hash.slice(1))?.classList.contains('win')) openWin(location.hash.slice(1), { scroll: false });

  // ---------------------------------------------------------------- start menu, tidy, shut down
  const start = $('#start'), menu = $('#startmenu');
  const setMenu = (open) => { menu.hidden = !open; start.setAttribute('aria-expanded', String(open)); if (open) $('button, a', menu)?.focus(); };
  start.addEventListener('click', (e) => { e.stopPropagation(); setMenu(menu.hidden); });
  document.addEventListener('click', (e) => { if (!menu.hidden && !menu.contains(e.target)) setMenu(false); });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !menu.hidden) { setMenu(false); start.focus(); } });
  menu.addEventListener('click', (e) => {
    const b = e.target.closest('[data-win-id]');
    if (b) { setMenu(false); openWin(b.dataset.winId); }
  });
  $('#tidy').addEventListener('click', () => {
    for (const w of wins) { w.classList.remove('closed', 'minimized', 'maximized'); w.style.transform = ''; w.dataset.x = 0; w.dataset.y = 0; }
    setMenu(false); sync(); scrollTo({ top: 0, behavior: reduced ? 'auto' : 'smooth' });
  });
  $('#shutdown').addEventListener('click', () => { setMenu(false); document.body.classList.add('off'); $('#shutdown-screen').hidden = false; $('#poweron').focus(); });
  $('#poweron').addEventListener('click', () => { document.body.classList.remove('off'); $('#shutdown-screen').hidden = true; start.focus(); });

  // ---------------------------------------------------------------- clock + GitHub stars
  const clock = $('#clock');
  const tick = () => { const d = new Date(); clock.textContent = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }); clock.dateTime = d.toISOString(); };
  tick(); setInterval(tick, 15000);

  async function stars() {
    let n = null;
    try {
      const c = JSON.parse(localStorage.getItem('aw.stars') || 'null');
      if (c && Date.now() - c.t < 3600e3) n = c.n;
    } catch { /* storage unavailable */ }
    if (n == null) {
      try {
        const r = await fetch(`https://api.github.com/repos/${REPO}`);
        if (r.ok) { n = (await r.json()).stargazers_count; try { localStorage.setItem('aw.stars', JSON.stringify({ n, t: Date.now() })); } catch { /* */ } }
      } catch { /* offline or rate limited */ }
    }
    const digits = n == null ? '------' : String(n).padStart(6, '0');
    $('#stars').innerHTML = [...digits].map((d) => `<i>${d}</i>`).join('');
    $('#stars').setAttribute('aria-label', n == null ? 'unknown' : `${n}`);
    $('#tray-stars').textContent = n == null ? '–' : String(n);
  }
  stars();

  // ---------------------------------------------------------------- install: OS detection, tabs, copy
  const ua = (navigator.userAgentData?.platform || navigator.platform || navigator.userAgent || '').toLowerCase();
  const os = /win/.test(ua) ? 'win' : /mac|iphone|ipad/.test(ua) ? 'mac' : /linux|x11|cros/.test(ua) ? 'linux' : null;
  const tabFor = { win: 't-win', mac: 't-nix', linux: 't-nix' };
  const label = { win: 'for Windows', mac: 'for macOS', linux: 'for Linux' };
  const dl = $('#dl');
  $('b', dl).textContent = 'Install';
  $('#dl-os').textContent = label[os] || 'Windows · macOS · Linux';
  dl.addEventListener('click', (e) => { e.preventDefault(); if (os) selectTab(document.getElementById(tabFor[os])); openWin('install'); });

  const tabs = $$('[role=tab]');
  function selectTab(t) {
    for (const x of tabs) { const on = x === t; x.setAttribute('aria-selected', String(on)); x.tabIndex = on ? 0 : -1; document.getElementById(x.getAttribute('aria-controls')).hidden = !on; }
  }
  for (const t of tabs) {
    t.addEventListener('click', () => selectTab(t));
    t.addEventListener('keydown', (e) => {
      const i = tabs.indexOf(t);
      const n = e.key === 'ArrowRight' ? tabs[(i + 1) % tabs.length] : e.key === 'ArrowLeft' ? tabs[(i - 1 + tabs.length) % tabs.length] : null;
      if (n) { e.preventDefault(); selectTab(n); n.focus(); }
    });
  }
  if (os) selectTab(document.getElementById(tabFor[os]));

  // direct downloads under each one-liner
  const asset = { 'p-win': [['agentwatch-x86_64-pc-windows-msvc.zip', 'Windows x64 .zip']], 'p-nix': [['agentwatch-aarch64-apple-darwin.tar.gz', 'macOS Apple Silicon'], ['agentwatch-x86_64-apple-darwin.tar.gz', 'macOS Intel'], ['agentwatch-x86_64-unknown-linux-gnu.tar.gz', 'Linux x64']] };
  for (const [panel, files] of Object.entries(asset)) {
    const p = document.createElement('p');
    p.className = 'note';
    p.innerHTML = 'Or download the binary: ' + files.map(([f, l]) => `<a href="https://github.com/${REPO}/releases/latest/download/${f}">${l}</a>`).join(' · ') + ` (<a href="https://github.com/${REPO}/releases/latest/download/SHA256SUMS">checksums</a>)`;
    document.getElementById(panel).appendChild(p);
  }

  for (const b of $$('.copy')) {
    b.addEventListener('click', async () => {
      const text = b.previousElementSibling.textContent;
      try { await navigator.clipboard.writeText(text); }
      catch { const ta = document.createElement('textarea'); ta.value = text; document.body.appendChild(ta); ta.select(); document.execCommand('copy'); ta.remove(); }
      b.textContent = 'Copied!'; b.classList.add('done');
      setTimeout(() => { b.textContent = 'Copy'; b.classList.remove('done'); }, 1800);
    });
  }

  // ---------------------------------------------------------------- picture viewer
  const thumbs = $$('.thumbs button');
  const viewer = $('#viewer');
  let cur = 0;
  const show = (i) => {
    cur = (i + thumbs.length) % thumbs.length;
    const t = thumbs[cur], img = $('img', t);
    $('#viewer-img').src = t.dataset.full; $('#viewer-img').alt = img.alt;
    $('#viewer-cap').textContent = `${cur + 1} / ${thumbs.length}: ${$('span', t).textContent}`;
    $('#viewer-title').textContent = `${$('span', t).textContent} - Picture Viewer`;
  };
  thumbs.forEach((t, i) => t.addEventListener('click', () => { show(i); viewer.showModal?.() ?? viewer.setAttribute('open', ''); }));
  $('#viewer-prev').addEventListener('click', () => show(cur - 1));
  $('#viewer-next').addEventListener('click', () => show(cur + 1));
  $('#viewer-close').addEventListener('click', () => viewer.close());
  viewer.addEventListener('click', (e) => { if (e.target === viewer) viewer.close(); });
  viewer.addEventListener('keydown', (e) => { if (e.key === 'ArrowLeft') show(cur - 1); if (e.key === 'ArrowRight') show(cur + 1); });

  // ---------------------------------------------------------------- demo terminal (Typed.js, real agentwatch output)
  const STATUS = /\b(running|thinking|stale|stuck|done)\b/g;
  const paint = (s) => esc(s).replace(STATUS, '<span class="k-$1">$1</span>').replace(/`/g, "'");
  const instant = (html) => '`' + html + '`';
  const prompt = '<span class="p">PS C:\\code\\acme-shop&gt;</span> ';
  async function script() {
    let d;
    try { d = await (await fetch('assets/demo-output.json')).json(); } catch { d = null; }
    if (!d) return [instant(prompt + 'agentwatch status\n(demo output unavailable)')];
    const head = (s, n) => s.split('\n').slice(0, n).join('\n');
    // Typed.js only honours a backtick block after a typed character, so every ^pause is followed by a newline.
    const NL = '\n';
    return [
      instant(prompt) + 'agentwatch status^400' + NL
      + instant(paint(d.status) + NL + NL + prompt) + '^700claude^500' + NL
      + instant(NL + '<span class="dim">╭─ Claude Code · acme-shop ─────────────────────────╮</span>' + NL + '<span class="p">&gt;</span> ')
      + 'how are my agents doing? anything broken?^600' + NL
      + instant(NL + '<span class="k-tool">●</span> agentwatch_status <span class="dim">(MCP)</span>' + NL + '<span class="dim">' + paint(head(d.mcpStatus, 3)) + '</span>' + NL) + '^500' + NL
      + instant('<span class="k-tool">●</span> agentwatch_errors <span class="dim">(MCP)</span>' + NL + '<span class="dim">' + paint(head(d.mcpErrors, 2)) + '</span>' + NL) + '^700' + NL
      + instant('<span class="p">●</span> ') + 'Nothing is broken. One failing test run was fixed (recovered) and one production deploy was blocked by your guard, as intended. I will wait for the rest with agentwatch_wait.',
    ];
  }
  let typed = null, started = false;
  const term = $('#typed');
  async function run() {
    started = true;
    const strings = await script();
    typed?.destroy();
    if (reduced || !window.Typed) { term.innerHTML = strings[0].replace(/`/g, '').replace(/\^\d+/g, ''); return; }
    typed = new window.Typed(term, { strings, typeSpeed: 38, startDelay: 300, contentType: 'html', cursorChar: '█', loop: false });
  }
  $('#replay').addEventListener('click', run);
  if ('IntersectionObserver' in window) {
    const io = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting) && !started) { io.disconnect(); run(); } }, { threshold: 0.3 });
    io.observe($('#terminal'));
  } else run();

  sync();
})();
