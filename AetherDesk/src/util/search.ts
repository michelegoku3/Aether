/**
 * Reusable fuzzy search and text normalization utilities.
 * Shared across Home, Library, and other views to uphold DRY and consistency.
 */

/**
 * Normalizes input text by lowercasing, stripping diacritics / accents,
 * converting punctuation to spaces, and trimming.
 */
export const normalizeSearchText = (value: string): string =>
  value
    .toLowerCase()
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, ' ')
    .trim();

/**
 * Calculates a fuzzy match score between a search query and candidate string.
 * Lower score = better match. Returns Infinity if no match is found.
 */
export const fuzzyScore = (query: string, candidate: string): number => {
  const q = normalizeSearchText(query);
  const c = normalizeSearchText(candidate);

  if (!q) return 0;
  if (c === q) return 0;
  if (c.startsWith(q)) return 1 + c.length - q.length;
  if (c.includes(q)) return 100 + (c.indexOf(q) * 2) + c.length - q.length;

  let lastIndex = -1;
  let score = 500;
  for (const char of q) {
    const index = c.indexOf(char, lastIndex + 1);
    if (index === -1) return Number.POSITIVE_INFINITY;
    score += index - lastIndex;
    lastIndex = index;
  }

  return score + c.length - q.length;
};

/**
 * Evaluates the best match score for a game considering both name and optional appId.
 */
export const matchGameScore = (query: string, name: string, appId?: string): number => {
  const trimmed = query.trim();
  if (!trimmed) return 0;

  const normalizedQuery = normalizeSearchText(trimmed);

  // Direct App ID match or App ID prefix / substring match
  if (appId) {
    const cleanAppId = appId.trim();
    if (cleanAppId === trimmed) return 1;
    if (cleanAppId.startsWith(trimmed)) return 2;
    if (cleanAppId.includes(trimmed)) return 50;
  }

  return fuzzyScore(normalizedQuery, name);
};

/**
 * Filters and sorts game items by relevance against a search query.
 * When query is empty, returns all items sorted alphabetically by name.
 */
export const filterAndSortGames = <T extends { name: string; appId?: string }>(
  items: readonly T[],
  query: string,
): T[] => {
  const trimmed = query.trim();
  if (!trimmed) {
    return [...items].sort((a, b) => a.name.localeCompare(b.name));
  }

  const scored: Array<{ item: T; score: number }> = [];
  for (const item of items) {
    const score = matchGameScore(trimmed, item.name, item.appId);
    if (Number.isFinite(score)) {
      scored.push({ item, score });
    }
  }

  scored.sort((a, b) => {
    if (a.score !== b.score) {
      return a.score - b.score;
    }
    return a.item.name.localeCompare(b.item.name);
  });

  return scored.map((s) => s.item);
};
