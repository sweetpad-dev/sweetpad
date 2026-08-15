// Dotted version handling for the tools SweetPad shells out to. Deliberately
// not a semver library: the only question asked is "is this new enough", and a
// dependency for one comparison would cost more than it saves.

const VERSION_REGEX = /(\d+)\.(\d+)\.(\d+)/;

/**
 * The first `major.minor.patch` in `text`, or undefined when there is none.
 *
 * Takes the first match so a whole `--version` line works as input, and so a
 * build that stamps itself `0.1.5-dev+<sha>` reads as 0.1.5 — a local or
 * pre-release build of a version speaks the same protocol as the release.
 */
export function parseVersion(text: string): [number, number, number] | undefined {
  const match = VERSION_REGEX.exec(text);
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : undefined;
}

/** Whether `actual` is `minimum` or newer. Unparseable input is not. */
export function isVersionAtLeast(actual: string, minimum: string): boolean {
  const a = parseVersion(actual);
  const b = parseVersion(minimum);
  if (a === undefined || b === undefined) return false;
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] > b[i];
  }
  return true;
}
