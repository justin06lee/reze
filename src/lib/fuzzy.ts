/**
 * Subsequence fuzzy match with positional bonuses, tuned for short trigger
 * names — "fa" should land on "full analysis" ahead of "refactor plan".
 */

export type Match = {
  score: number;
  /** Indices in the target that were matched, for highlighting. */
  indices: number[];
};

const SEPARATORS = new Set([" ", "-", "_", "/", ".", ":"]);

export function fuzzyMatch(query: string, target: string): Match | null {
  if (!query) return { score: 0, indices: [] };

  const q = query.toLowerCase();
  const t = target.toLowerCase();
  const indices: number[] = [];

  let score = 0;
  let ti = 0;
  let lastMatch = -2;

  for (let qi = 0; qi < q.length; qi++) {
    const ch = q[qi];
    // Spaces in the query are separators, not literals — "fu an" still matches
    // "full analysis" without the user typing the exact gap.
    if (ch === " ") continue;

    let found = -1;
    for (let i = ti; i < t.length; i++) {
      if (t[i] === ch) {
        found = i;
        break;
      }
    }
    if (found === -1) return null;

    if (found === 0) score += 20;
    else if (SEPARATORS.has(t[found - 1])) score += 14;
    if (found === lastMatch + 1) score += 10;

    // Mild penalty for gaps, so tight matches float up without swamping bonuses.
    score -= Math.min(found - ti, 6);

    indices.push(found);
    lastMatch = found;
    ti = found + 1;
  }

  // Reward a clean prefix or exact hit above anything the per-char bonuses give.
  const compact = q.replace(/\s+/g, "");
  if (t === compact) score += 60;
  else if (t.startsWith(compact)) score += 30;
  else if (t.includes(compact)) score += 15;

  // Shorter targets are a better fit for the same match.
  score -= t.length * 0.1;

  return { score, indices };
}
