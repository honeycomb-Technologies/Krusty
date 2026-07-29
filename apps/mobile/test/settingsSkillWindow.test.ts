import {
  clampSkillPageStart,
  nextSkillPageStart,
  previousSkillPageStart,
  SKILL_PAGE_SIZE,
} from "../components/settings/skillWindow";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown): void {
  if (actual !== expected) {
    throw new Error(`Expected ${String(expected)}, received ${String(actual)}`);
  }
}

Deno.test("skills settings bounds the first native render", () => {
  assertEquals(clampSkillPageStart(0, 100), 0);
  assertEquals(Math.min(SKILL_PAGE_SIZE, 100), SKILL_PAGE_SIZE);
  assertEquals(clampSkillPageStart(0, 0), 0);
});

Deno.test("skills settings navigation never accumulates mounted rows", () => {
  let start = 0;
  for (let index = 0; index < 100; index += 1) {
    start = nextSkillPageStart(start, 100);
    const mountedCount = Math.min(SKILL_PAGE_SIZE, 100 - start);
    if (mountedCount > SKILL_PAGE_SIZE) {
      throw new Error(`Mounted ${mountedCount} skill rows`);
    }
  }
  assertEquals(start, 96);
  assertEquals(previousSkillPageStart(start), 88);
});
