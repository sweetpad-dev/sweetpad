export function uniqueFilter<T>(value: T, index: number, array: T[]): boolean {
  return array.indexOf(value) === index;
}
