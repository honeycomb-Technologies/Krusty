import { createWorkerDmNavigationFence } from "../components/hive/workerDmNavigationFence.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function source(path: string): Promise<string> {
  return Deno.readTextFile(new URL(path, import.meta.url));
}

Deno.test("mobile Hive drawer exposes Workers and Groups before either exists", async () => {
  const drawer = await source("../components/chat/SessionDrawer.tsx");

  const managementRows = drawer.indexOf(
    "...hiveManagementDestinations.map((destination)",
  );
  const conditionalWorkers = drawer.indexOf("...(hiveWorkers.length > 0");
  assert(
    managementRows >= 0 && managementRows < conditionalWorkers,
    "management destinations must be unconditional and precede roster rows",
  );
  assert(
    drawer.includes('item.id === "crew" || item.id === "groups"') &&
      drawer.includes("accessibilityLabel={isWorkers") &&
      drawer.includes('"Manage Hive Workers"') &&
      drawer.includes('"Manage Hive Groups"') &&
      drawer.includes("onSelectHiveView?.(destination.id)"),
    "both management rows must be accessible and route through the Hive view contract",
  );
  assert(
    drawer.includes('if (item.kind === "worker")') &&
      drawer.includes('if (item.kind === "group")') &&
      drawer.includes("<HiveGroupRoomView"),
    "existing Worker DMs, Group rows, and the Group room must remain intact",
  );
});

Deno.test("mobile Hive management replaces the stable thread surface instead of stacking it", async () => {
  const [index, mobileControls] = await Promise.all([
    source("../app/(tabs)/index.tsx"),
    source("../components/hive/HiveMobileThreadControls.tsx"),
  ]);
  const compact = index.replace(/\s+/g, " ");

  assert(
    compact.includes(
      'const showMobileHiveManagement = activeMode === "hive" && hiveTopLevel !== "hive";',
    ) &&
      compact.includes(
        ": showMobileHiveManagement ? hiveContent : showMobileHiveThreadTransition ? mobileHiveThreadTransition : mobileContent",
      ),
    "only a non-thread Hive destination may replace the mobile conversation surface",
  );
  assert(
    compact.includes(
      "{chatTranscriptSurface} </View> </View> </GestureDetector>",
    ) && compact.includes("<HiveMobileThreadControls") &&
      compact.includes("primaryComposer={sharedComposer}") &&
      !mobileControls.includes("<ChatTranscript"),
    "Hive home and Worker DMs must keep one transcript while selecting exact lower controls",
  );
  assert(
    compact.includes(
      "onOpenMenu={!isDesktop ? () => setDrawerOpen(true) : undefined}",
    ),
    "mobile management must retain a route back to the drawer",
  );
});

Deno.test("Workers and Groups mount exact management views and DMs wait for their exact thread", async () => {
  const [screen, navigation, index] = await Promise.all([
    source("../components/hive/HiveScreen.tsx"),
    source("../components/hive/hooks/useHiveNavigation.ts"),
    source("../app/(tabs)/index.tsx"),
  ]);

  assert(
    screen.includes("useHiveNavigation(requestedTopLevel)") &&
      navigation.includes(
        'initialTopLevel: HiveTopLevelView = "hive"',
      ) && navigation.includes(
        "useState<HiveTopLevelView>(initialTopLevel)",
      ),
    "a freshly mounted management surface must not render Hive home first",
  );
  assert(
    screen.includes('navigation.topLevel === "crew"') &&
      screen.includes("<HiveCrewView") &&
      screen.includes('navigation.topLevel === "groups"') &&
      screen.includes("<HiveGroupsView"),
    "Workers and Groups must reuse their existing full management views",
  );

  const crewView = screen.slice(
    screen.indexOf("<HiveCrewView"),
    screen.indexOf("/>", screen.indexOf("<HiveCrewView")) + 2,
  );
  assert(
    screen.includes("onOpenWorkerDm: (sessionId: string) => void") &&
      crewView.includes("onOpenWorkerDm={onOpenWorkerDm}") &&
      !crewView.includes('navigation.setTopLevel("hive");'),
    "the management surface must stay mounted while a Worker DM selection is pending",
  );

  assert(
    index.includes("setPendingHiveThreadSessionId(targetSessionId)") &&
      index.includes("void loadSessionById(targetSessionId)") &&
      index.includes('if (requestedMode !== "hive")') &&
      index.includes(
        "[activeMode, pendingHiveThreadSessionId, requestedMode, sessionId]",
      ) &&
      index.includes("sessionId !== pendingHiveThreadSessionId") &&
      index.includes('setHiveTopLevel("hive")') &&
      index.includes("onSelectHiveSession={handleOpenHiveWorkerDm}") &&
      index.includes('accessibilityLabel="Opening Hive Worker conversation"'),
    "management and cross-mode drawer Worker selections must preserve the fence until the exact target shell is active",
  );
});

Deno.test("late Worker DM navigation is fenced by ownership and newer intent", () => {
  const fence = createWorkerDmNavigationFence();
  fence.mount();

  const workerA = fence.beginIntent();
  assert(
    fence.isCurrent(workerA),
    "the initiating Worker intent must own navigation",
  );

  const workerB = fence.beginIntent();
  assert(
    !fence.isCurrent(workerA) && fence.isCurrent(workerB),
    "a newer Worker intent must supersede a late Worker A response",
  );

  fence.invalidate();
  assert(
    !fence.isCurrent(workerB),
    "drawer close, mode changes, and newer navigation must invalidate the owner",
  );

  const workerC = fence.beginIntent();
  fence.unmount();
  assert(
    !fence.isCurrent(workerC),
    "an unmounted Worker surface must reject every late continuation",
  );
});

Deno.test("Crew and drawer gate every late Worker-to-DM continuation", async () => {
  const [crew, drawer] = await Promise.all([
    source("../components/hive/HiveCrewView.tsx"),
    source("../components/chat/SessionDrawer.tsx"),
  ]);

  assert(
    crew.includes("workerDmNavigationFenceRef.current.beginIntent()") &&
      crew.includes("workerDmNavigationFenceRef.current.isCurrent(intent)") &&
      crew.includes("fence.unmount()") &&
      crew.includes("workers.ensureWorkerDm(worker.id)") &&
      crew.includes(".retryIntroduction(worker.id)") &&
      crew.includes("workers.createWorker(request)"),
    "Crew ensure, Introduction retry, and create continuations must share one mounted intent fence",
  );
  assert(
    drawer.includes("workerDmNavigationFenceRef.current.beginIntent()") &&
      drawer.includes("workerDmNavigationFenceRef.current.isCurrent(intent)") &&
      drawer.includes("invalidateWorkerDmNavigation();") &&
      drawer.includes("onClose={closeDrawer}") &&
      drawer.includes(
        "[activeMode, client, invalidateWorkerDmNavigation, isOpen]",
      ) &&
      drawer.includes("client.ensureHiveWorkerDm(worker.id)"),
    "the drawer ensure response must be fenced across close, mode/client changes, and newer navigation",
  );
});
