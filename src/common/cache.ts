/**
 * Caches the result of a function call based on its arguments.
 */
export function cache<T, Args extends unknown[]>(fn: (...args: Args) => Promise<T>): {
  (...args: Args): Promise<T>;
  clearCache: () => void;
} {
  const cache: Record<string, T> = {};

  const cachedFn = async (...args: Args): Promise<T> => {
    const key = JSON.stringify(args);
    if (key in cache) {
      return cache[key];
    }
    const result = await fn(...args);
    cache[key] = result;
    return result;
  };

  cachedFn.clearCache = () => {
    Object.keys(cache).forEach(key => delete cache[key]);
  };

  return cachedFn;
}
