import { Timer } from "./timer";

describe("Timer", () => {
  it("starts with zero or near-zero elapsed time", () => {
    const timer = new Timer();
    expect(timer.elapsed).toBeLessThan(50);
  });

  it("measures elapsed time", async () => {
    const timer = new Timer();
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(timer.elapsed).toBeGreaterThanOrEqual(80);
    expect(timer.elapsed).toBeLessThan(300);
  });
});
