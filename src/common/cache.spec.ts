import { cache } from "./cache";

describe("cache", () => {
  it("returns cached result on second call with same args", async () => {
    let callCount = 0;
    const fn = cache(async (x: number) => {
      callCount++;
      return x * 2;
    });

    expect(await fn(5)).toBe(10);
    expect(await fn(5)).toBe(10);
    expect(callCount).toBe(1);
  });

  it("calls function again for different args", async () => {
    let callCount = 0;
    const fn = cache(async (x: number) => {
      callCount++;
      return x * 2;
    });

    expect(await fn(5)).toBe(10);
    expect(await fn(3)).toBe(6);
    expect(callCount).toBe(2);
  });

  it("caches based on multiple args", async () => {
    let callCount = 0;
    const fn = cache(async (a: number, b: string) => {
      callCount++;
      return `${a}-${b}`;
    });

    expect(await fn(1, "a")).toBe("1-a");
    expect(await fn(1, "a")).toBe("1-a");
    expect(await fn(1, "b")).toBe("1-b");
    expect(callCount).toBe(2);
  });

  it("clearCache resets the cache", async () => {
    let callCount = 0;
    const fn = cache(async (x: number) => {
      callCount++;
      return x * 2;
    });

    expect(await fn(5)).toBe(10);
    expect(callCount).toBe(1);

    fn.clearCache();

    expect(await fn(5)).toBe(10);
    expect(callCount).toBe(2);
  });

  it("handles zero-arg functions", async () => {
    let callCount = 0;
    const fn = cache(async () => {
      callCount++;
      return 42;
    });

    expect(await fn()).toBe(42);
    expect(await fn()).toBe(42);
    expect(callCount).toBe(1);
  });
});
