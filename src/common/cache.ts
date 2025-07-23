/**
 * Caches the result of a function call based on its arguments
 */
export function cache<T, Args extends unknown[]>(
  fn: (...args: Args) => Promise<T>,
): {
  (...args: Args): Promise<T>;
  clearCache: () => void;
} {
  const cache = new Map<string, T>();

  const cachedFn = async (...args: Args): Promise<T> => {
    const key = JSON.stringify(args);
    if (cache.has(key)) {
      return cache.get(key) as T;
    }
    const result = await fn(...args);
    cache.set(key, result);
    return result;
  };

  cachedFn.clearCache = () => cache.clear();
  return cachedFn;
}
