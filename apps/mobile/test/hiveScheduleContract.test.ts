declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("Calendar keeps Hive run titles read-only and uses only Hive timing control", async () => {
  const schedule = await Deno.readTextFile(
    new URL("../components/hive/HiveScheduleView.tsx", import.meta.url),
  );
  const titleFieldStart = schedule.indexOf("Run title");
  const titleFieldEnd = schedule.indexOf("Detail", titleFieldStart);
  const titleField = schedule.slice(titleFieldStart, titleFieldEnd);

  assert(
    !schedule.includes("updateSession"),
    "Calendar must never send Hive-owned titles through generic session metadata",
  );
  assert(
    titleFieldStart >= 0 &&
      titleFieldEnd > titleFieldStart &&
      titleField.includes("{target.item.title}") &&
      !titleField.includes("<TextInput") &&
      !schedule.includes("onChangeTitle") &&
      !schedule.includes("detailTitle"),
    "Calendar must present the current run title honestly as read-only",
  );
  assert(
    schedule.includes("client.scheduleHiveSession(") &&
      schedule.includes("schedule.startAt !== detailTarget.item.wakeAt"),
    "Calendar must retain its typed Hive wake-time mutation",
  );
});
