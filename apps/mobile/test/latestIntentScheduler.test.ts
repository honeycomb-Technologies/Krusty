// @ts-ignore Deno requires an explicit extension; the Expo compiler resolves
// the same source through its extensionless application imports.
import { createLatestIntentScheduler } from "../components/navigation/latestIntentScheduler.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals<T>(actual: T, expected: T, message: string): void {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

class FakeClock {
  nowMs = 0;
  private nextId = 1;
  private timers = new Map<number, { dueAtMs: number; callback: () => void }>();

  readonly now = () => this.nowMs;

  readonly setTimer = (callback: () => void, delayMs: number): number => {
    const id = this.nextId++;
    this.timers.set(id, { dueAtMs: this.nowMs + delayMs, callback });
    return id;
  };

  readonly clearTimer = (id: number): void => {
    this.timers.delete(id);
  };

  advanceBy(deltaMs: number): void {
    const targetMs = this.nowMs + deltaMs;
    while (true) {
      const next = [...this.timers.entries()]
        .filter(([, timer]) => timer.dueAtMs <= targetMs)
        .sort((left, right) => left[1].dueAtMs - right[1].dueAtMs || left[0] - right[0])[0];
      if (!next) break;
      const [id, timer] = next;
      this.timers.delete(id);
      this.nowMs = timer.dueAtMs;
      timer.callback();
    }
    this.nowMs = targetMs;
  }
}

function createHarness(quietDelayMs = 100, maxDelayMs = 250) {
  const clock = new FakeClock();
  const flushed: Array<{ value: string; atMs: number }> = [];
  const scheduler = createLatestIntentScheduler<string, number>({
    quietDelayMs,
    maxDelayMs,
    now: clock.now,
    setTimer: clock.setTimer,
    clearTimer: clock.clearTimer,
    onFlush: (value) => flushed.push({ value, atMs: clock.nowMs }),
  });
  return { clock, flushed, scheduler };
}

Deno.test("latest intent scheduler admits only the latest intent after quiet", () => {
  const { clock, flushed, scheduler } = createHarness();

  scheduler.submit("chat");
  clock.advanceBy(60);
  scheduler.submit("code");
  clock.advanceBy(99);
  assertEquals(flushed.length, 0, "intent must remain pending before quiet delay");
  clock.advanceBy(1);

  assertEquals(flushed.length, 1, "one coalesced intent should flush");
  assertEquals(flushed[0].value, "code", "superseded intent must be discarded");
  assertEquals(flushed[0].atMs, 160, "quiet delay must follow latest submission");
  assert(!scheduler.hasPending(), "scheduler must be empty after automatic flush");
});

Deno.test("latest intent scheduler enforces its first-intent hard deadline", () => {
  const { clock, flushed, scheduler } = createHarness();

  scheduler.submit("0");
  for (const value of ["80", "160", "240"]) {
    clock.advanceBy(80);
    scheduler.submit(value);
  }
  clock.advanceBy(9);
  assertEquals(flushed.length, 0, "continuous input may coalesce before hard limit");
  clock.advanceBy(1);

  assertEquals(flushed.length, 1, "hard limit must force admission");
  assertEquals(flushed[0].value, "240", "hard-limit flush must use latest intent");
  assertEquals(flushed[0].atMs, 250, "hard limit must stay anchored to burst start");
});

Deno.test("manual flush admits immediately and invalidates the scheduled callback", () => {
  const { clock, flushed, scheduler } = createHarness();

  scheduler.submit("mako");
  clock.advanceBy(25);
  assert(scheduler.flush(), "flush must report admitted pending work");
  assertEquals(flushed[0].atMs, 25, "manual flush must be immediate");
  assert(!scheduler.flush(), "flush must report false once empty");
  clock.advanceBy(500);
  assertEquals(flushed.length, 1, "cleared timer must not flush a second time");
});

Deno.test("cancel drops pending work and a new burst receives a new hard deadline", () => {
  const { clock, flushed, scheduler } = createHarness();

  scheduler.submit("old");
  clock.advanceBy(90);
  assert(scheduler.cancel(), "cancel must report dropped pending work");
  assert(!scheduler.cancel(), "cancel must report false once empty");
  clock.advanceBy(100);
  scheduler.submit("new");
  clock.advanceBy(100);

  assertEquals(flushed.length, 1, "only the new burst should flush");
  assertEquals(flushed[0].value, "new", "cancelled intent must never be admitted");
  assertEquals(flushed[0].atMs, 290, "new burst must use its own quiet deadline");
});
