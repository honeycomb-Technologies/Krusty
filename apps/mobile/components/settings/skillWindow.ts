export const SKILL_PAGE_SIZE = 8;

export function clampSkillPageStart(
  requestedStart: number,
  skillCount: number,
): number {
  if (skillCount <= 0) return 0;
  const lastPageStart =
    Math.floor((skillCount - 1) / SKILL_PAGE_SIZE) * SKILL_PAGE_SIZE;
  return Math.min(lastPageStart, Math.max(0, requestedStart));
}

export function nextSkillPageStart(
  currentStart: number,
  skillCount: number,
): number {
  return clampSkillPageStart(currentStart + SKILL_PAGE_SIZE, skillCount);
}

export function previousSkillPageStart(currentStart: number): number {
  return Math.max(0, currentStart - SKILL_PAGE_SIZE);
}
