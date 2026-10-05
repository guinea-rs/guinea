const HEIGHT = 180;

const rng = (seed) => () => {
  seed |= 0;
  seed = seed + 0x6d2b79f5 | 0;
  let t = Math.imul(seed ^ seed >>> 15, 1 | seed);
  t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t;
  return ((t ^ t >>> 14) >>> 0) / 4294967296;
};

const rgb = (hex) => [
  parseInt(hex.slice(1, 3), 16),
  parseInt(hex.slice(3, 5), 16),
  parseInt(hex.slice(5, 7), 16),
];

const canvas = (w, h) => {
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  return c;
};

const painter = (c) => {
  const ctx = c.getContext('2d');
  const img = ctx.createImageData(c.width, c.height);
  const { width: W, height: H } = c;
  const cache = {};

  return {
    set(x, y, hex) {
      if (x < 0 || y < 0 || x >= W || y >= H) return;
      const [r, g, b] = cache[hex] || (cache[hex] = rgb(hex));
      const i = (y * W + x) * 4;
      img.data[i] = r;
      img.data[i + 1] = g;
      img.data[i + 2] = b;
      img.data[i + 3] = 255;
    },
    done: () => ctx.putImageData(img, 0, 0),
  };
};

const ridge = (W, base, amp, skew, freq, r) => {
  const p = [r() * 6.28, r() * 6.28, r() * 6.28];
  const out = [];
  for (let x = 0; x <= W; x++) {
    const f = 0.55 * Math.sin(x * 0.045 * freq + p[0])
      + 0.3 * Math.sin(x * 0.11 * freq + p[1])
      + 0.15 * Math.sin(x * 0.23 * freq + p[2]);
    const m = 1 - skew * (1 - x / W);
    out.push(Math.round(base - amp * m * (f + 1) / 2));
  }
  return out;
};

const litAt = (line, x, W) => line[Math.min(W, x + 4)] >= line[Math.max(0, x - 4)];

const sky = (cfg) => {
  const { W, H } = cfg;
  const c = canvas(W, H);
  const p = painter(c);

  const bands = ['#8fcbe0', '#a4d5e4', '#bbdfe8', '#d2e9ea', '#e8f1e6'];
  const bh = cfg.far.base / bands.length;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      let b = Math.min(bands.length - 1, Math.floor(y / bh));
      const into = y - b * bh;
      if (b < bands.length - 1 && into > bh - 2 && (x + y) % 2 === 0) b++;
      p.set(x, y, bands[b]);
    }
  }

  const [sx, sy] = cfg.sun;
  for (let y = -12; y <= 12; y++) {
    for (let x = -12; x <= 12; x++) {
      const d = x * x + y * y;
      if (d <= 49) p.set(sx + x, sy + y, '#fff3c4');
      else if (d <= 90 && (x + y) % 2 === 0) p.set(sx + x, sy + y, '#fbf4d6');
    }
  }

  p.done();
  return c;
};

const land = (cfg, r) => {
  const { W, H } = cfg;
  const c = canvas(W, H);
  const p = painter(c);

  const haze = ridge(W, cfg.far.base - Math.round(cfg.far.amp * 0.35), cfg.far.amp * 0.9, cfg.far.skew * 0.5, 0.7, r);
  for (let x = 0; x < W; x++) {
    const lit = litAt(haze, x, W);
    for (let y = haze[x]; y < H; y++) {
      p.set(x, y, y < cfg.snow - 6 ? (lit ? '#eef0f3' : '#d5dbe8') : (lit ? '#bcc6e0' : '#adb8d6'));
    }
  }

  const far = ridge(W, cfg.far.base, cfg.far.amp, cfg.far.skew, 1, r);
  const snowEdge = Array.from({ length: W }, (_, x) =>
    cfg.snow + Math.round(2.2 * Math.sin(x * 0.37 + 1.3) + 1.5 * Math.sin(x * 0.91)));
  const gully = Array.from({ length: W }, () => r() < 0.09);
  for (let x = 0; x < W; x++) {
    const lit = litAt(far, x, W);
    const edge = far[x] < far[Math.max(0, x - 1)] || far[x] < far[Math.min(W, x + 1)];
    for (let y = far[x]; y < H; y++) {
      const depth = y - far[x];
      let snow = y < snowEdge[x] || (y === snowEdge[x] && x % 2 === 0);
      if (!snow && gully[x] && lit && y < snowEdge[x] + 7 && (y + x) % 2 === 0) snow = true;

      let col = snow ? (lit ? '#f7f3ea' : '#d6dbe8') : (lit ? '#a3afd3' : '#8893c0');
      if (!snow && !lit && depth > 2 && (x * 7 + y * 3) % 23 === 0) col = '#7a85b3';
      if (!snow && lit && depth > 3 && (x * 5 + y * 11) % 29 === 0) col = '#b3bddc';
      if (depth === 0 && !snow) col = lit ? '#b6c0de' : '#9aa4cc';
      if (depth === 0 && snow && edge) col = '#ffffff';
      p.set(x, y, col);
    }
  }

  const mid = ridge(W, cfg.mid.base, cfg.mid.amp, 0, 1.6, r);
  for (let x = 0; x < W; x++) {
    const lit = litAt(mid, x, W);
    for (let y = mid[x]; y < H; y++) {
      const depth = y - mid[x];
      let col = lit ? '#8db592' : '#779f80';
      if (depth === 0) col = lit ? '#a2c49f' : '#88ad8c';
      else if (depth > 1 && (x * 13 + y * 7) % 17 === 0) col = lit ? '#7fa885' : '#6a9274';
      p.set(x, y, col);
    }
  }

  const back = ridge(W, cfg.hill - 2, 5, 0, 1.9, r);
  for (let x = 0; x < W; x++) {
    const lit = litAt(back, x, W);
    for (let y = back[x]; y < H; y++) {
      p.set(x, y, y === back[x] ? '#a8c86a' : lit ? '#93b65c' : '#86a954');
    }
  }

  const hill = ridge(W, cfg.hill + 1, 3, 0, 2.4, r);
  for (let x = 0; x < W; x++) {
    for (let y = hill[x]; y < H; y++) {
      p.set(x, y, y === hill[x] ? '#b4d16e' : (x + y) % 9 === 0 ? '#93b456' : '#9fc05f');
    }
  }

  const mh = H - cfg.meadow;
  for (let y = cfg.meadow; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const n = r();
      const t = (y - cfg.meadow) / mh;
      const base = t < 0.3 ? '#adcd60' : t < 0.7 ? '#a3c55a' : '#97bb52';
      p.set(x, y, y === cfg.meadow ? '#bad873' : n < 0.05 ? '#88ac4a' : n < 0.08 ? '#bdd976' : base);
    }
  }

  for (let i = 0; i < W / 2.5; i++) {
    const bx = Math.floor(r() * W);
    const by = cfg.meadow + 1 + Math.floor(r() * (mh - 1));
    p.set(bx, by, '#86aa48');
    p.set(bx, by - 1, '#93b650');
  }

  const stamp = (rows, x0, y0, colours) =>
    rows.forEach((row, yy) => [...row].forEach((ch, xx) => {
      if (colours[ch]) p.set(x0 + xx, y0 + yy, colours[ch]);
    }));

  for (let i = 0; i < W / 40; i++) {
    const x = Math.floor(r() * W);
    const y = cfg.hill + 3 + Math.floor(r() * (H - cfg.hill - 5));
    stamp(['.ss.', 'sSSs'], x, y, { s: '#a7a79a', S: '#8e8e82' });
  }

  for (let i = 0; i < Math.round(W / 11); i++) {
    const x = Math.floor(r() * W);
    const y = cfg.hill + 2 + Math.floor(r() * (H - cfg.hill - 6));
    stamp(['.y.y.', 'yy.yy', '.yyy.', 'zzzzz'], x, y, { y: '#dcc47c', z: '#b89c55' });
  }

  for (let i = 0; i < W / 6; i++) {
    const x = Math.floor(r() * W);
    const y = cfg.meadow + 2 + Math.floor(r() * (H - cfg.meadow - 3));
    p.set(x, y, r() < 0.5 ? '#fbf6ea' : '#f2c14e');
  }

  p.done();
  return c;
};

const BODY = [
  '........pp......',
  '....ggggppgg....',
  '..ggcccggggggg..',
  '.ggcccccggggkgc.',
  'gggccccggggggccc',
  'gggggggggggggccp',
  'ggggggggggggcccc',
  '.ggggggggggcccc.',
  '.ggggddddgggcc..',
];

const STRIDE = [
  { dy: 0, feet: '.pp.........pp..' },
  { dy: 0, feet: '..pp......pp....' },
  { dy: -1, feet: '....pp..pp......' },
  { dy: 0, feet: '..pp......pp....' },
];

const COATS = [
  { g: '#d98a4a', d: '#c77a3e', c: '#f6e6d2', k: '#2a2622', p: '#e08a8f' },
  { g: '#f3e6cf', d: '#e3d3b7', c: '#d98a4a', k: '#2a2622', p: '#e08a8f' },
  { g: '#3d3430', d: '#332b27', c: '#f6e6d2', k: '#120e0c', p: '#e08a8f' },
  { g: '#a06e47', d: '#8f613d', c: '#f6e6d2', k: '#2a2622', p: '#e08a8f' },
];

const sprites = () => COATS.map((coat) => [false, true].map((flip) => STRIDE.map((step) => {
  const c = canvas(16, 16);
  const p = painter(c);
  const put = (row, line) => [...line].forEach((ch, x) => {
    if (coat[ch]) p.set(flip ? 15 - x : x, row, coat[ch]);
  });

  BODY.forEach((line, i) => put(3 + step.dy + i, line));
  put(12 + step.dy, step.feet);
  p.done();
  return c;
})));

const layout = (W) => ({
  W,
  H: HEIGHT,
  seed: 3,
  far: { base: 118, amp: 36, skew: 0.6 },
  snow: 96,
  mid: { base: 134, amp: 10 },
  hill: 141,
  meadow: 147,
  sun: [W - 58, 36],
  clouds: Math.max(2, Math.round(W / 80)),
  pigs: Math.max(2, Math.round(W / 64)),
  lanes: [156, 175],
});

const herd = (cfg) => Array.from({ length: cfg.pigs }, (_, i) => ({
  x: Math.random() * (cfg.W - 16),
  y: Math.round(cfg.lanes[0] + (cfg.lanes[1] - cfg.lanes[0]) * (i + 0.5) / cfg.pigs),
  dir: Math.random() < 0.5 ? 1 : -1,
  speed: 11 + Math.random() * 6,
  state: Math.random() < 0.6 ? 'run' : 'idle',
  t: Math.random() * 3,
  phase: Math.random() * 4,
  coat: i % COATS.length,
}));

const step = (scene, dt) => {
  const { W } = scene.cfg;
  scene.time += dt;

  for (const cloud of scene.clouds) {
    cloud.x += cloud.s * dt;
    if (cloud.x > W + 4) cloud.x = -cloud.w - 4;
  }

  for (const pig of scene.pigs) {
    pig.t -= dt;
    if (pig.state === 'run') {
      pig.x += pig.dir * pig.speed * dt;
      if (pig.x < -2) pig.dir = 1;
      if (pig.x > W - 14) pig.dir = -1;
      if (pig.t <= 0) {
        pig.state = 'idle';
        pig.t = 0.8 + Math.random() * 2.4;
      }
    } else if (pig.t <= 0) {
      pig.state = 'run';
      pig.t = 1.5 + Math.random() * 3;
      if (Math.random() < 0.45) pig.dir *= -1;
    }
  }
};

const draw = (scene, ctx, coats) => {
  ctx.drawImage(scene.sky, 0, 0);

  for (const { x: cx, y, w } of scene.clouds) {
    const x = Math.round(cx);
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(x + Math.round(w * 0.22), y, Math.round(w * 0.36), 2);
    ctx.fillRect(x + Math.round(w * 0.55), y + 1, Math.round(w * 0.24), 1);
    ctx.fillRect(x, y + 2, w, 2);
    ctx.fillStyle = '#e1eef2';
    ctx.fillRect(x + 1, y + 4, w - 2, 1);
  }

  ctx.drawImage(scene.land, 0, 0);

  for (const pig of [...scene.pigs].sort((a, b) => a.y - b.y)) {
    const x = Math.round(pig.x);
    const frame = pig.state === 'run' ? Math.floor(scene.time * 10 + pig.phase) % 4 : 1;
    ctx.fillStyle = 'rgba(62, 86, 32, .28)';
    ctx.fillRect(x + 3, pig.y + 1, 10, 1);
    ctx.drawImage(coats[pig.coat][pig.dir < 0 ? 1 : 0][frame], x, pig.y - 12);
  }
};

const widthFor = (el) => {
  const { width, height } = el.getBoundingClientRect();
  if (!width || !height) return 320;
  return Math.max(160, Math.min(480, Math.round(HEIGHT * width / height)));
};

export function mountMeadow(el) {
  const ctx = el.getContext('2d');
  const coats = sprites();
  const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let scene;

  const build = () => {
    const cfg = layout(widthFor(el));
    const r = rng(cfg.seed);
    el.width = cfg.W;
    el.height = cfg.H;
    ctx.imageSmoothingEnabled = false;

    scene = {
      cfg,
      sky: sky(cfg),
      land: land(cfg, r),
      clouds: Array.from({ length: cfg.clouds }, () => ({
        x: r() * cfg.W,
        y: 6 + Math.floor(r() * cfg.far.base * 0.4),
        w: 14 + Math.floor(r() * 16),
        s: 1.5 + r() * 1.5,
      })),
      pigs: herd(cfg),
      time: 0,
    };
    draw(scene, ctx, coats);
  };

  build();

  let pending = 0;
  new ResizeObserver(() => {
    clearTimeout(pending);
    pending = setTimeout(() => {
      if (widthFor(el) !== scene.cfg.W) build();
    }, 120);
  }).observe(el);

  if (still) return;

  let last = performance.now();
  const loop = (now) => {
    const dt = Math.max(0, Math.min(0.1, (now - last) / 1000));
    last = now;
    step(scene, dt);
    draw(scene, ctx, coats);
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
}
