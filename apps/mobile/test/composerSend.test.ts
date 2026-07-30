import { resolveComposerSendPayload } from "../components/chat/composerSend";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("restored text remains part of an immediate send", () => {
  const restoredTextRef = { current: " restored draft " };
  const payload = resolveComposerSendPayload(restoredTextRef.current, []);
  assert(payload?.content === "restored draft", "restored text must be sent");
});

Deno.test("attachment-only sends remain valid", () => {
  const attachment = { uri: "file:///image.jpg" };
  const payload = resolveComposerSendPayload("", [attachment]);
  assert(payload?.attachments?.[0] === attachment, "attachment should be retained");
});
