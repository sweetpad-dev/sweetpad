export function isNotNull<T>(value: T | null): value is T {
  return value !== null;
}

// Use this literal instead of "never" to ensure that a switch statement is exhaustive.
// See issue: https://github.com/microsoft/TypeScript/issues/41707
type NeverStringLiteral = "__THIS_SHOULD_BE_UNREACHABLE__";

export function assertUnreachable(value: NeverStringLiteral): never {
  throw new Error(`Unreachable: ${value}`);
}

// Like assertUnreachable, but does not throw an error
export function checkUnreachable(value: NeverStringLiteral): void {
  // do nothing
}
