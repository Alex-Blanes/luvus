// Generates public/og.png (1200×630), the social card for luvus.dev.
//
// Static asset, rendered here so it stays in step with the brand: the logo
// (public/logo.png, the white tile, rounded like an app icon and sized
// prominently), the
// noir palette (src/styles/themes.css :root), and JetBrains Mono (the site's
// mono face; must be installed for fontconfig — `fc-list | grep -i jetbrains` —
// with a `monospace` fallback so a machine without it still renders sanely).
// Re-run after a logo/brand change:  node scripts/make-og.mjs
import sharp from 'sharp';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');

// noir (:root) tokens — keep in sync with src/styles/themes.css.
const C = {
  bg: '#050507',
  card: '#0c0c11',
  border: '#1c1c22',
  text: '#e7e7ed',
  sub: '#a2a2ae', // subtitle — bright enough to read on the card
  accent: '#c6ff1a', // lime
  green: '#8fbc7a', // done
  dim: '#4a4a54', // idle dot
  install: '#70707c', // install hint — subtle but legible
};

const W = 1200;
const H = 630;
const FONT = 'JetBrains Mono, monospace';
const ADV = 0.6; // JetBrains Mono advance ≈ 0.6em, used for width math

// A status pill: dot + "agent · state". Monospace, so width is char-count based.
function chip(x, y, agent, state, dot) {
  const label = `${agent} · ${state}`;
  const fs = 24;
  const padL = 26,
    padR = 26,
    dotGap = 16,
    dotR = 6;
  const w = padL + dotR * 2 + dotGap + label.length * fs * ADV + padR;
  const h = 52;
  const cy = y + h / 2;
  return {
    w,
    svg: `
      <g>
        <rect x="${x}" y="${y}" width="${w}" height="${h}" rx="12" fill="#141419" stroke="${C.border}"/>
        <circle cx="${x + padL + dotR}" cy="${cy}" r="${dotR}" fill="${dot}"/>
        <text x="${x + padL + dotR * 2 + dotGap}" y="${cy}" font-family="${FONT}" font-size="${fs}"
              font-weight="500" fill="#b6b6c0" dominant-baseline="central">${label}</text>
      </g>`,
  };
}

// Lay the three chips left-to-right with a gap.
const chipY = 508;
const gap = 20;
let cx = 80;
const c1 = chip(cx, chipY, 'claude', 'idle', C.dim);
cx += c1.w + gap;
const c2 = chip(cx, chipY, 'codex', 'working', C.accent);
cx += c2.w + gap;
const c3 = chip(cx, chipY, 'copilot', 'done', C.green);

const svg = `
<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <radialGradient id="glow" cx="88%" cy="6%" r="72%">
      <stop offset="0%" stop-color="${C.accent}" stop-opacity="0.13"/>
      <stop offset="55%" stop-color="${C.accent}" stop-opacity="0"/>
    </radialGradient>
  </defs>

  <rect width="${W}" height="${H}" fill="${C.bg}"/>
  <rect x="20" y="20" width="${W - 40}" height="${H - 40}" rx="28" fill="${C.card}" stroke="${C.border}"/>
  <rect x="20" y="20" width="${W - 40}" height="${H - 40}" rx="28" fill="url(#glow)"/>

  <!-- wordmark (logo composited by sharp at 76×76 at x=64,y=60 → centre y=98) -->
  <text x="160" y="98" font-family="${FONT}" font-size="38" font-weight="700"
        fill="${C.text}" dominant-baseline="central">Luvus</text>
  <text x="${W - 64}" y="98" font-family="${FONT}" font-size="28" font-weight="600"
        fill="${C.accent}" text-anchor="end" dominant-baseline="central">luvus.dev</text>

  <!-- headline -->
  <text x="80" y="256" font-family="${FONT}" font-size="68" font-weight="800" fill="${C.text}">Mission control for</text>
  <text x="80" y="342" font-family="${FONT}" font-size="68" font-weight="800" fill="${C.text}">your <tspan fill="${C.accent}">AI coding agents</tspan></text>

  <!-- subtitle, wrapped so it never crowds the card edge -->
  <text x="82" y="424" font-family="${FONT}" font-size="27" font-weight="400" fill="${C.sub}">Run, watch, resume, and orchestrate every</text>
  <text x="82" y="462" font-family="${FONT}" font-size="27" font-weight="400" fill="${C.sub}">coding agent from one terminal.</text>

  ${c1.svg}${c2.svg}${c3.svg}

  <!-- install hint -->
  <text x="82" y="590" font-family="${FONT}" font-size="21" font-weight="400" fill="${C.install}">$ brew install RizRiyz/luvus/luvus</text>
</svg>`;

// The mark: full-bleed square art, rounded like an app icon (22% radius), sized
// up from the old card so it reads clearly.
const LOGO = 76;
const radius = Math.round(LOGO * 0.22);
const mask = Buffer.from(
  `<svg width="${LOGO}" height="${LOGO}"><rect width="${LOGO}" height="${LOGO}" rx="${radius}" ry="${radius}"/></svg>`,
);
const logo = await sharp(join(root, 'public/logo.png'))
  .resize(LOGO, LOGO, { fit: 'cover' })
  .composite([{ input: mask, blend: 'dest-in' }])
  .png()
  .toBuffer();

await sharp(Buffer.from(svg))
  .composite([{ input: logo, left: 64, top: 60 }])
  .png()
  .toFile(join(root, 'public/og.png'));

console.log('wrote public/og.png (1200×630)');
