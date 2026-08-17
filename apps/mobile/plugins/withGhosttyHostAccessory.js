const fs = require("fs");
const path = require("path");

const ANCHOR = "    addSubview(terminalView)\n";
const PATCH = `    addSubview(terminalView)
    // Mitsuro supplies TerminalQuickBar. Empty items hide Ghostty's keyboard accessory.
    #if !targetEnvironment(macCatalyst)
    terminalView.inputAccessoryItems = []
    #endif
`;

function applyGhosttyHostAccessory(projectRoot) {
  const file = path.join(
    projectRoot,
    "node_modules/expo-libghostty/ios/ExpoLibghosttyView.swift",
  );
  if (!fs.existsSync(file)) {
    return { applied: false, reason: "missing" };
  }

  const source = fs.readFileSync(file, "utf8");
  if (source.includes("inputAccessoryItems = []")) {
    return { applied: false, reason: "already" };
  }
  if (!source.includes(ANCHOR)) {
    throw new Error(
      "expo-libghostty ExpoLibghosttyView.swift no longer contains the addSubview anchor; cannot hide the native keyboard accessory",
    );
  }

  fs.writeFileSync(file, source.replace(ANCHOR, PATCH));
  return { applied: true, reason: "patched" };
}

function withGhosttyHostAccessory(config) {
  const { withDangerousMod } = require("expo/config-plugins");
  return withDangerousMod(config, [
    "ios",
    async (mod) => {
      applyGhosttyHostAccessory(mod.modRequest.projectRoot);
      return mod;
    },
  ]);
}

module.exports = withGhosttyHostAccessory;
module.exports.applyGhosttyHostAccessory = applyGhosttyHostAccessory;

if (require.main === module) {
  applyGhosttyHostAccessory(process.cwd());
}
