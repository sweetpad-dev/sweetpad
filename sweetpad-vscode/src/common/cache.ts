/**
 * Caches the result of a function call based on its arguments
 */
export function cache<T, Args extends unknown[]>(
  fn: (...args: Args) => Promise<T>,
): {
  (...args: Args): Promise<T>;
  clearCache: () => void;
} {
  const store = new Map<string, T>();

  const cachedFn = async (...args: Args): Promise<T> => {
    const key = JSON.stringify(args);
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
