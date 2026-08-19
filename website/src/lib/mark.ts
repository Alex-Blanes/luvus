import raw from '../assets/luvus-logo.svg?raw';

/**
 * The nav mark, retinted so it takes its colour from CSS.
 *
 * The canonical source has dark ink on a white backing. Strip only that backing
 * and map its ink and knockout fills onto the active theme.
 *
 * Mapping the two colours onto `currentColor` and `var(--bg)` keeps the drawing
 * exactly as drawn, while letting whatever sets `color` on the lockup drive the
 * mark and the wordmark together.
 *
 * Exported from here rather than inlined at each call site because it was
 * inlined at each call site, the landing pages missed an edit to the artwork,
 * and the two navs drifted apart.
 */
export const NAV_MARK = raw
  .replace(/<rect width="1000" height="1000" fill="white"\/>/, '')
  .replace(/#272727/gi, 'currentColor')
  .replace(/fill="white"/g, 'fill="var(--bg)"');
