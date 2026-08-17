declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("plan tracker is compact and anchored above the composer", async () => {
  const [surface, tracker, transcript, desktopShell] = await Promise.all([
    Deno.readTextFile(
      new URL(
        "../components/chat-screen/ActiveConversationSurface.tsx",
        import.meta.url,
      ),
    ),
    Deno.readTextFile(
      new URL("../components/chat/PlanTracker.tsx", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../components/chat/ChatTranscript.tsx", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../components/layout/DesktopShell.tsx", import.meta.url),
    ),
  ]);

  assert(
    surface.includes("bottom: bottomPadding + 8") &&
      surface.includes(
        "const conversationBottomPadding = bottomPadding + planTrackerHeight +",
      ) &&
      surface.includes("planTrackerGap;") &&
      surface.includes("const planTrackerRightInset = 12 + 56 + 10") &&
      surface.includes("right: planTrackerRightInset"),
    "conversation surface must anchor the tracker above the measured composer, reserve its height, and clear the Agent FAB column",
  );
  assert(
    tracker.includes(
      "const [goalExpanded, setGoalExpanded] = useState(false)",
    ) &&
      tracker.includes(
        "const [planExpanded, setPlanExpanded] = useState(false)",
      ),
    "goal and plan must each default to their small collapsed title",
  );
  assert(
    tracker.includes('"Collapse goal"') &&
      tracker.includes('"Expand goal"') &&
      tracker.includes('accessibilityLabel="Collapse goal"') &&
      tracker.includes('"Collapse plan"') &&
      tracker.includes('"Expand plan"') &&
      tracker.includes('accessibilityLabel="Collapse plan"'),
    "goal and plan must expose independent expand and collapse controls",
  );
  assert(
    tracker.includes(
      "const trackerAvailable = goalAvailable || planAvailable",
    ) &&
      tracker.includes("const planAvailable =") &&
      tracker.includes("workflow?.plan_revision"),
    "a goal must remain visible even before a plan exists",
  );
  assert(
    tracker.includes("const sectionControlCount =") &&
      tracker.includes("<View style={styles.controlRow}>") &&
      tracker.includes("controlRow: {") &&
      tracker.includes('flexDirection: "row"') &&
      tracker.includes('justifyContent: "center"') &&
      tracker.includes("styles.collapsedChipShared") &&
      tracker.includes("styles.collapsedChipSolo"),
    "goal and plan controls must stay in a bottom row and center a lone control",
  );
  assert(
    tracker.indexOf("goalAvailable && goalExpanded") <
        tracker.indexOf("<View style={styles.controlRow}>") &&
      tracker.indexOf("planAvailable && planExpanded") <
        tracker.indexOf("<View style={styles.controlRow}>") &&
      tracker.includes(
        'onPress={() => setSectionExpanded("goal", !goalExpanded)}',
      ) &&
      tracker.includes(
        'onPress={() => setSectionExpanded("plan", !planExpanded)}',
      ),
    "expanded content must open above the persistent goal and plan controls",
  );
  assert(
    tracker.includes("SUCCESS CRITERIA"),
    "goal bullets must be identified as success criteria",
  );
  assert(
    tracker.includes("<ScrollView") &&
      tracker.includes("contentContainerStyle={styles.items}") &&
      tracker.includes("nestedScrollEnabled") &&
      tracker.includes("showsVerticalScrollIndicator={false}"),
    "the bounded plan step list must scroll without a visible native scrollbar",
  );
  assert(
    tracker.includes("const approved = await executeWorkflowCommand") &&
      tracker.includes('"activate_goal"') &&
      tracker.includes('"resume_goal"') &&
      tracker.includes('? "Start"'),
    "starting a proposed plan must approve it and continue into execution",
  );
  assert(
    surface.includes("hidePlanTracker?: boolean") &&
      surface.includes("Animated.timing(planTrackerOpacity") &&
      surface.includes("duration: hidePlanTracker ? 90 : 140") &&
      surface.includes('pointerEvents={hidePlanTracker ? "none" : "box-none"}') &&
      surface.includes("if (hidePlanTracker) setPlanTrackerHeight(0)") &&
      surface.includes("setPlanTrackerMounted(false)") &&
      surface.includes("showPlanTracker && planTrackerMounted"),
    "the plan tracker must fade, drop its reserved height, and unmount so FABs return to their original alignment",
  );
  assert(
    !transcript.includes("import { PlanTracker }") &&
      !desktopShell.includes("import { PlanTracker }"),
    "the tracker must not return to transcript-top or sidebar ownership",
  );
});
