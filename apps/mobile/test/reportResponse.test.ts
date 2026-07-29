import { reportSummariesFromResponse } from "../components/reports/reportResponse";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("malformed report responses degrade to an empty toolbox", () => {
  assert(reportSummariesFromResponse(null).length === 0, "null must be empty");
  assert(reportSummariesFromResponse({}).length === 0, "missing reports must be empty");
  assert(
    reportSummariesFromResponse({ reports: "not-an-array" }).length === 0,
    "non-array reports must be empty",
  );
});

Deno.test("valid report arrays preserve their exact identity", () => {
  const reports = [{
    id: "fixture-report",
    title: "Fixture",
    summary: "Fixture summary",
    tags: [],
    created_at: "2026-07-29T00:00:00Z",
  }];

  assert(
    reportSummariesFromResponse({ reports }) === reports,
    "a valid typed response must not allocate or transform its report list",
  );
});
