/**
 * Measures elapsed time in milliseconds.
 */
export class Timer {
  private startTime: number;
  constructor() {
    this.startTime = Date.now();
  }
  get elapsed() {
    return Date.now() - this.startTime;
  }
}
