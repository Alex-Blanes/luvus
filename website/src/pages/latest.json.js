// Emitted at build time as `/latest.json` — the small version manifest the
// luvus app's background update check reads to learn the newest release. It is
// derived from the same `changelog/*.md` files the changelog page uses, so it
// can never drift from what is actually published. Notify-only on the app side:
// the app shows a dot by its version and points at the upgrade command; it does
// not self-update.

const files = import.meta.glob('../../../changelog/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
});

// Pull the `version` / `date` front matter and the note body out of a file.
const parse = (path, text) => {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  const meta = {};
  if (m) {
    for (const line of m[1].split(/\r?\n/)) {
      const kv = /^(\w+):\s*(.*)$/.exec(line.trim());
      if (kv) meta[kv[1]] = kv[2];
    }
  }
  const tag = meta.version || (path.split('/').pop() || '').replace(/\.md$/, '');
  return {
    version: tag.replace(/^v/, ''),
    date: meta.date || '',
    notes: (m ? text.slice(m[0].length) : text).trim(),
  };
};

// Rank versions numerically so v0.10.0 sorts above v0.9.0.
const rank = (v) =>
  v.replace(/^v/, '').split('.').map(Number).reduce((a, n) => a * 10000 + (n || 0), 0);

const entries = Object.entries(files)
  .map(([path, text]) => parse(path, text))
  .sort((a, b) => rank(b.version) - rank(a.version));

export const GET = () => {
  const latest = entries[0] || { version: '0.0.0', date: '', notes: '' };
  return new Response(JSON.stringify(latest, null, 2) + '\n', {
    headers: { 'Content-Type': 'application/json; charset=utf-8' },
  });
};
