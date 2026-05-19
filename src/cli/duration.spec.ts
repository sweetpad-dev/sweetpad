import { parseDuration } from "./duration";

describe("cli/duration parseDuration", () => {
  it("parses bare integer seconds", () => {
    expect(parseDuration("30")).toBe(30);
    expect(parseDuration("0")).toBe(0);
  });

  it("parses bare decimal seconds", () => {
    expect(parseDuration("1.5")).toBe(1.5);
  });

  it("parses single-unit suffixes", () => {
    expect(parseDuration("30s")).toBe(30);
    expect(parseDuration("5m")).toBe(300);
    expect(parseDuration("1h")).toBe(3600);
  });

  it("parses composite forms", () => {
    expect(parseDuration("1h30m")).toBe(5400);
    expect(parseDuration("2h15m10s")).toBe(8110);
    expect(parseDuration("90m")).toBe(5400);
  });

  it("returns undefined for invalid input", () => {
    expect(parseDuration("")).toBeUndefined();
    expect(parseDuration("abc")).toBeUndefined();
    expect(parseDuration("30x")).toBeUndefined();
    expect(parseDuration("m30")).toBeUndefined();
  });

  it("trims whitespace", () => {
    expect(parseDuration("  30s  ")).toBe(30);
  });
});
