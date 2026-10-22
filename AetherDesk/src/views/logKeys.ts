export interface KeyedLine {
  key: string;
  line: string;
}

/**
 * Assigns React keys to log lines.
 *
 * The backend sends plain text without per-line ids, and the tail window
 * shifts as old lines leave, so array indices cannot be keys: a poll that
 * returns unchanged text would remount every row. Content is stable, but
 * duplicate lines (e.g. the same startup message twice) would collide, so the
 * key is the content plus how many identical lines precede it. Appending a
 * line leaves every earlier key untouched; a duplicate only shifts the keys of
 * later copies of that same text.
 */
export const stableLineKeys = (lines: string[]): KeyedLine[] => {
  const seen = new Map<string, number>();

  return lines.map((line) => {
    const occurrence = seen.get(line) ?? 0;
    seen.set(line, occurrence + 1);
    // NUL cannot appear in a log line, so the separator keeps the occurrence
    // prefix unambiguous for any content.
    return { key: `${occurrence}\u0000${line}`, line };
  });
};
