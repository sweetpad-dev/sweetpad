/**
 * Caches the result of a function call based on a derived key.
 *
 * Pass a `keyFn` whenever any of the function's arguments contain non-JSON-safe
 * values (class instances with timer/OutputChannel refs, circular structures,
 * functions). When omitted, the default key is `JSON.stringify(args)` — fine
 * for plain-data args, will throw on anything containing a `Timeout`, a vscode
 * `OutputChannel`, etc.
 */
export function cache<T, Args extends unknown[]>(
  fn: (...args: Args) => Promise<T>,
  keyFn?: (...args: Args) => string,
): {
  (...args: Args): Promise<T>;
  clearCache: () => void;
} {
  const store = new Map<string, T>();
  const computeKey = keyFn ?? ((...args: Args) => JSON.stringify(args));

  const cachedFn = async (...args: Args): Promise<T> => {
    const key = computeKey(...args);
    if (store.has(key)) {
      return store.get(key) as T;
    }
    const result = await fn(...args);
    store.set(key, result);
    return result;
  };

  cachedFn.clearCache = () => store.clear();
  return cachedFn;
}
