declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("terminal controls center a bounded reactive puck and stay keyboard aware", async () => {
  const [bar, webTerminal, nativeTerminal, accessoryPlugin, appConfig, packageJson] =
    await Promise.all([
      Deno.readTextFile(
        new URL("../components/toolbox/TerminalQuickBar.tsx", import.meta.url),
      ),
      Deno.readTextFile(
        new URL("../components/desktop/Terminal.tsx", import.meta.url),
      ),
      Deno.readTextFile(
        new URL("../components/toolbox/ToolboxTerminal.tsx", import.meta.url),
      ),
      Deno.readTextFile(
        new URL("../plugins/withGhosttyHostAccessory.js", import.meta.url),
      ),
      Deno.readTextFile(new URL("../app.json", import.meta.url)),
      Deno.readTextFile(new URL("../package.json", import.meta.url)),
    ]);

  for (
    const control of [
      "Paste into terminal",
      "Send Control C",
      "Send Escape",
      "Send Tab",
      "Terminal directional control",
      "Send Enter",
      "Clear terminal",
    ]
  ) {
    assert(bar.includes(control), `quick bar must include ${control}`);
  }

  assert(
    bar.includes("<ClipboardPaste") &&
      bar.includes('label: "⇥"') &&
      bar.includes('label: "↵"') &&
      !bar.includes(">Paste</Text>"),
    "paste must be icon-only and common terminal actions should use compact symbols",
  );
  assert(
    bar.includes("styles.controlsRow") &&
      bar.includes("styles.leftCluster") &&
      bar.includes("styles.rightCluster") &&
      bar.includes("<TerminalDirectionPad") &&
      !bar.includes("<ScrollView") &&
      !bar.includes("borderWidth") &&
      !bar.includes("shadowOpacity"),
    "symbol clusters must balance around one centered puck without nested chrome",
  );
  assert(
    bar.includes('pointerEvents="box-none"') &&
      bar.includes('position: "absolute"') &&
      bar.includes("maxWidth: 400") &&
      bar.includes("minWidth: 0") &&
      bar.includes("measureInWindow") &&
      bar.includes("endCoordinates.screenY") &&
      bar.includes("{ bottom: keyboardLift }"),
    "controls must float over the terminal and compress safely at phone widths",
  );
  assert(
    bar.includes("Gesture.Pan()") &&
      bar.includes("<GestureDetector") &&
      bar.includes("useSharedValue") &&
      bar.includes("useAnimatedStyle") &&
      bar.includes("withSpring") &&
      !bar.includes("PanResponder"),
    "direction puck must use the native Gesture Handler and Reanimated pipeline",
  );
  assert(
    bar.includes("DIRECTION_HOLD_DELAY_MS = 320") &&
      bar.includes("DIRECTION_REPEAT_MS = 150") &&
      bar.includes("setTimeout(tick, DIRECTION_REPEAT_MS)") &&
      bar.includes("clearTimeout") &&
      bar.includes("onFinalize") &&
      !bar.includes("setInterval"),
    "held direction must tick steadily without interval catch-up or uncancelled repeats",
  );
  assert(
    bar.includes("window.visualViewport") &&
      bar.includes("Keyboard.addListener") &&
      bar.includes("measureInWindow") &&
      bar.includes("endCoordinates.screenY"),
    "quick keys must sit on the keyboard using host overlap, not raw keyboard height",
  );
  assert(
    webTerminal.includes("suppressGhosttyScrollbar(terminal)") &&
      webTerminal.includes("terminal.scrollbarOpacity = 0") &&
      webTerminal.includes("<TerminalQuickBar") &&
      webTerminal.includes("terminalRef.current.paste(text)"),
    "web Ghostty must keep scrollback while hiding its canvas scrollbar and supporting paste",
  );
  assert(
    nativeTerminal.includes("<TerminalQuickBar") &&
      nativeTerminal.includes("const sendQuickInput") &&
      nativeTerminal.includes("Clipboard.getStringAsync()"),
    "native Ghostty must share the same quick input and paste controls",
  );
  assert(
    accessoryPlugin.includes("inputAccessoryItems = []") &&
      accessoryPlugin.includes("addSubview(terminalView)") &&
      appConfig.includes("./plugins/withGhosttyHostAccessory") &&
      packageJson.includes("node ./plugins/withGhosttyHostAccessory.js"),
    "native Ghostty must hide its keyboard accessory so only the Mitsuro pill sits on the keyboard",
  );
});
