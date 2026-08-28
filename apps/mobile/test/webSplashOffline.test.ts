declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("web splash keeps its animation runtime inside the app bundle", async () => {
  const webSplash = await Deno.readTextFile(
    new URL("../components/splash/SplashOverlay.web.tsx", import.meta.url),
  );

  assert(
    webSplash.includes("MitsuroTraceMark"),
    "web splash must render the local Mitsuro trace",
  );
  assert(
    !webSplash.includes("lottie-react-native") &&
      !webSplash.includes("@lottiefiles"),
    "web splash must not instantiate a runtime that fetches remote WASM",
  );
  assert(
    !webSplash.includes("http://") && !webSplash.includes("https://"),
    "web splash must not contain an off-origin runtime URL",
  );
});
