import { expect, test } from "bun:test";
import {
  hasClosedHtmlFence,
  isHtmlPreviewLanguage,
} from "../apps/mobile/components/chat/htmlPreviewModel.ts";

test("recognizes only explicit HTML fence languages", () => {
  expect(isHtmlPreviewLanguage("html")).toBe(true);
  expect(isHtmlPreviewLanguage("HTM title=demo")).toBe(true);
  expect(isHtmlPreviewLanguage("javascript")).toBe(false);
  expect(isHtmlPreviewLanguage("")).toBe(false);
});

test("waits for a matching closing fence before rendering HTML", () => {
  expect(hasClosedHtmlFence("```html\n<h1>Hello</h1>\n```")).toBe(true);
  expect(hasClosedHtmlFence("~~~~html\n<h1>Hello</h1>\n~~~~")).toBe(true);
  expect(hasClosedHtmlFence("```html\n<h1>Streaming")).toBe(false);
  expect(hasClosedHtmlFence("````html\n<h1>Hello</h1>\n```")).toBe(false);
});
