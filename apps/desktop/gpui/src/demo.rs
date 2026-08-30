//! Static demo threads and transcript so the shell is usable offline
//! without live models or a running app-server.

use mitsuro_desktop_backend::ThreadSummary;

/// One transcript block in the main column (user, assistant, tools, plan, …).
#[derive(Clone, Debug)]
pub struct DemoMessage {
    pub kind: DemoMessageKind,
    /// App-server item id when streaming (for delta correlation).
    pub item_id: Option<String>,
    pub streaming: bool,
}

/// Discriminated content for Codex-like transcript blocks.
#[derive(Clone, Debug)]
pub enum DemoMessageKind {
    User {
        body: String,
    },
    Assistant {
        body: String,
    },
    Reasoning {
        body: String,
    },
    Plan {
        body: String,
    },
    CommandExecution {
        command: String,
        cwd: String,
        status: String,
        output: String,
    },
    FileChange {
        paths_summary: String,
        patch_preview: String,
        status: String,
    },
}

impl DemoMessage {
    pub fn user(body: impl Into<String>) -> Self {
        Self {
            kind: DemoMessageKind::User { body: body.into() },
            item_id: None,
            streaming: false,
        }
    }

    pub fn assistant(body: impl Into<String>) -> Self {
        Self {
            kind: DemoMessageKind::Assistant { body: body.into() },
            item_id: None,
            streaming: false,
        }
    }

    pub fn streaming_assistant(item_id: impl Into<String>) -> Self {
        Self {
            kind: DemoMessageKind::Assistant {
                body: String::new(),
            },
            item_id: Some(item_id.into()),
            streaming: true,
        }
    }

    pub fn reasoning(body: impl Into<String>, item_id: Option<String>) -> Self {
        Self {
            kind: DemoMessageKind::Reasoning { body: body.into() },
            item_id,
            streaming: false,
        }
    }

    pub fn plan(body: impl Into<String>, item_id: Option<String>) -> Self {
        Self {
            kind: DemoMessageKind::Plan { body: body.into() },
            item_id,
            streaming: false,
        }
    }

    pub fn command_execution(
        command: impl Into<String>,
        cwd: impl Into<String>,
        status: impl Into<String>,
        output: impl Into<String>,
        item_id: Option<String>,
    ) -> Self {
        Self {
            kind: DemoMessageKind::CommandExecution {
                command: command.into(),
                cwd: cwd.into(),
                status: status.into(),
                output: output.into(),
            },
            item_id,
            streaming: false,
        }
    }

    pub fn file_change(
        paths_summary: impl Into<String>,
        patch_preview: impl Into<String>,
        status: impl Into<String>,
        item_id: Option<String>,
    ) -> Self {
        Self {
            kind: DemoMessageKind::FileChange {
                paths_summary: paths_summary.into(),
                patch_preview: patch_preview.into(),
                status: status.into(),
            },
            item_id,
            streaming: false,
        }
    }

    /// Mutable primary text buffer for streaming deltas (body / output / patch).
    pub fn text_mut(&mut self) -> &mut String {
        match &mut self.kind {
            DemoMessageKind::User { body }
            | DemoMessageKind::Assistant { body }
            | DemoMessageKind::Reasoning { body }
            | DemoMessageKind::Plan { body } => body,
            DemoMessageKind::CommandExecution { output, .. } => output,
            DemoMessageKind::FileChange { patch_preview, .. } => patch_preview,
        }
    }

    /// Replace primary text (final item text / assembled body).
    pub fn set_text(&mut self, text: String) {
        *self.text_mut() = text;
    }

    #[allow(dead_code)]
    pub fn is_command(&self) -> bool {
        matches!(self.kind, DemoMessageKind::CommandExecution { .. })
    }

    #[allow(dead_code)]
    pub fn is_file_change(&self) -> bool {
        matches!(self.kind, DemoMessageKind::FileChange { .. })
    }

    /// Coarse role tag for simple match arms (mirrors [`DemoMessageKind`]).
    #[allow(dead_code)]
    pub fn role(&self) -> DemoRole {
        match &self.kind {
            DemoMessageKind::User { .. } => DemoRole::User,
            DemoMessageKind::Assistant { .. } => DemoRole::Assistant,
            DemoMessageKind::Reasoning { .. } => DemoRole::Reasoning,
            DemoMessageKind::Plan { .. } => DemoRole::Plan,
            DemoMessageKind::CommandExecution { .. } => DemoRole::CommandExecution,
            DemoMessageKind::FileChange { .. } => DemoRole::FileChange,
        }
    }
}

/// Coarse role tag for simple match arms (mirrors [`DemoMessageKind`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DemoRole {
    User,
    Assistant,
    Reasoning,
    Plan,
    CommandExecution,
    FileChange,
}

/// Which product surface a demo/local thread belongs to (Chat vs Codex agent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThreadSurface {
    /// Simplified ChatGPT-style chat (mode=chat).
    Chat,
    /// Agent threads — Codex mode (default / current main).
    #[default]
    Codex,
}

impl ThreadSurface {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DemoThread {
    pub summary: ThreadSummary,
    pub messages: Vec<DemoMessage>,
    /// Product surface filter (`mode=chat` vs Codex agent threads).
    pub surface: ThreadSurface,
}

/// Work-mode goal status (mirrors `ThreadGoalStatus` wire enum loosely).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)] // Paused/Blocked/Complete reserved for wire goal status mapping
pub enum DemoGoalStatus {
    #[default]
    Active,
    Paused,
    Blocked,
    Complete,
}

impl DemoGoalStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Complete => "complete",
        }
    }
}

/// One plan step under a long-running Work goal.
#[derive(Clone, Debug)]
pub struct DemoPlanItem {
    pub id: String,
    pub title: String,
    pub done: bool,
}

/// Long-running Work goal (fixture until `thread/goal/*` is fully wired).
#[derive(Clone, Debug)]
pub struct DemoGoal {
    pub id: String,
    pub objective: String,
    pub status: DemoGoalStatus,
    pub plan_items: Vec<DemoPlanItem>,
    /// Optional linked Codex/Chat thread id.
    pub thread_id: Option<String>,
    pub updated_at: Option<i64>,
}

/// Demo thread list matching Codex sidebar density (title + meta subtitle).
pub fn demo_threads() -> Vec<DemoThread> {
    vec![
        DemoThread {
            summary: ThreadSummary {
                id: "demo-1".into(),
                name: Some("Refactor GPUI shell chrome".into()),
                preview: Some("Match Codex layout: rail, sidebar, composer".into()),
                cwd: Some("~/Work/Mitsuro".into()),
                created_at: Some(1_722_700_000),
                updated_at: Some(1_722_701_200),
                model_provider: Some("openai".into()),
                ephemeral: Some(false),
                is_pinned: Some(true),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![
                DemoMessage::user(
                    "Sketch a Codex-like desktop shell in GPUI with dark tokens.",
                ),
                DemoMessage::reasoning(
                    "Match Codex open-thread density: title + path chip, muted reasoning, \
                     command cards, and a full composer without home promo stack.",
                    Some("reason-demo-1".into()),
                ),
                DemoMessage::plan(
                    "1. Thread header with title + project path\n\
                     2. Transcript: user/assistant + tool cards\n\
                     3. Composer: Full access · model · mic · send",
                    Some("plan-demo-1".into()),
                ),
                DemoMessage::assistant(
                    "Layout plan:\n• Thin activity rail (Chat / Work / Codex / Atlas / Terminal / Settings)\n• ~270px thread sidebar with search + New thread\n• Main transcript column\n• Rounded composer with model chip + send\n• Connection pill in the status bar\n\nUsing Codex CSS tokens for surfaces (#171717 / #212121) and white-alpha borders.",
                ),
                DemoMessage::command_execution(
                    "cargo build -p mitsuro-desktop",
                    "~/Work/Mitsuro",
                    "completed",
                    "   Compiling mitsuro-desktop v0.1.0\n    Finished `dev` profile [unoptimized + debuginfo] target(s) in 12.4s",
                    Some("cmd-demo-1".into()),
                ),
                DemoMessage::file_change(
                    "crates/mitsuro-desktop/src/components/main_column.rs",
                    "@@ -260,6 +260,18 @@\n+fn thread_title_bar(...)\n+    // title · path chip · overflow",
                    "completed",
                    Some("file-demo-1".into()),
                ),
                DemoMessage::user(
                    "Add static demo threads so we can UI-review without app-server.",
                ),
                DemoMessage::assistant(
                    "Demo data is wired. Selecting a thread shows a short transcript; New thread opens the empty-state prompt. Send replays the fixture turn stream.",
                ),
            ],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-2".into(),
                name: Some("Atlas browser surface".into()),
                preview: Some("Rail Atlas tab · BrowserPanel fixture".into()),
                cwd: Some("~/Work/Mitsuro".into()),
                created_at: Some(1_722_680_000),
                updated_at: Some(1_722_690_500),
                model_provider: Some("openai".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![
                DemoMessage::user("Where will Atlas / browser live in the chrome?"),
                DemoMessage::assistant(
                    "Activity rail middle icon opens the Atlas BrowserPanel: editable URL bar + Go, back/forward history via BrowserHost, Open external (xdg-open / Chromium --app sibling), profile discovery, mock page card with bridge status. wry links when browser-native is on; true child embed is blocked on Wayland/GTK — use MITSURO_ATLAS_EXTERNAL or MITSURO_ATLAS_SIBLING.",
                ),
            ],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-3".into(),
                name: Some("App-server protocol notes".into()),
                preview: Some("thread/start · thread/list · turn events".into()),
                cwd: Some("~/.codex".into()),
                created_at: Some(1_722_600_000),
                updated_at: Some(1_722_650_000),
                model_provider: Some("openai".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![
                DemoMessage::user(
                    "List the app-server methods we need for sidebar bootstrap.",
                ),
                DemoMessage::assistant(
                    "Minimum for shell list/resume: initialize, session list/start/read, plus turn streaming notifications.",
                ),
            ],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-4".into(),
                name: Some("Empty experiment".into()),
                preview: Some("No messages yet".into()),
                cwd: Some("~/Work/Mitsuro".into()),
                created_at: Some(1_722_550_000),
                updated_at: Some(1_722_550_100),
                model_provider: Some("openai".into()),
                ephemeral: Some(true),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-5".into(),
                name: Some("Core Fix".into()),
                preview: Some("Shell density pass".into()),
                cwd: Some("~/Work/Mitsuro".into()),
                created_at: Some(1_722_540_000),
                updated_at: Some(1_722_540_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(true),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![
                DemoMessage::user("Tighten the home shell."),
                DemoMessage::assistant("Sidebar nav + hero + composer stack updated."),
            ],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-6".into(),
                name: Some("Rename Mako to Hive and Mitsuro".into()),
                preview: Some("Branding sweep".into()),
                cwd: Some("~/Work/Mitsuro".into()),
                created_at: Some(1_722_530_000),
                updated_at: Some(1_722_530_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(true),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![DemoMessage::user("Rename pass."), DemoMessage::assistant("Done.")],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-7".into(),
                name: Some("Trading Research".into()),
                preview: Some("Notes".into()),
                cwd: None,
                created_at: Some(1_722_520_000),
                updated_at: Some(1_722_520_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![DemoMessage::user("Research notes."), DemoMessage::assistant("Stub.")],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-8".into(),
                name: Some("Tape".into()),
                preview: Some("Quick note".into()),
                cwd: None,
                created_at: Some(1_722_510_000),
                updated_at: Some(1_722_510_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(true),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        // Extra dense Recents (bar home fills the left rail)
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-9".into(),
                name: Some("Audit repo and set up domain".into()),
                preview: Some("Domain + DNS checklist".into()),
                cwd: Some("~/Work/sites".into()),
                created_at: Some(1_722_500_000),
                updated_at: Some(1_722_500_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-10".into(),
                name: Some("hey".into()),
                preview: Some("Quick hello".into()),
                cwd: None,
                created_at: Some(1_722_499_000),
                updated_at: Some(1_722_499_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(true),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-11".into(),
                name: Some("Audit repo and unblock Kline data".into()),
                preview: Some("Feed recovery".into()),
                cwd: Some("~/Work/trading".into()),
                created_at: Some(1_722_498_000),
                updated_at: Some(1_722_498_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-12".into(),
                name: Some("I've noticed a major issue with the bridge".into()),
                preview: Some("Sesori bridge notes".into()),
                cwd: Some("~/Work/sesori".into()),
                created_at: Some(1_722_497_000),
                updated_at: Some(1_722_497_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-13".into(),
                name: Some("I specifically want to use the Bitunix API".into()),
                preview: Some("Exchange integration".into()),
                cwd: None,
                created_at: Some(1_722_496_000),
                updated_at: Some(1_722_496_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-14".into(),
                name: Some("I want to create to folder in work, it's".into()),
                preview: Some("Scaffold".into()),
                cwd: None,
                created_at: Some(1_722_495_000),
                updated_at: Some(1_722_495_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(true),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-15".into(),
                name: Some("i want to create a browser game, we".into()),
                preview: Some("Game sketch".into()),
                cwd: None,
                created_at: Some(1_722_494_000),
                updated_at: Some(1_722_494_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(true),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-16".into(),
                name: Some("Investigate Honey Krusty Update".into()),
                preview: Some("Release notes".into()),
                cwd: Some("~/Work/krusty".into()),
                created_at: Some(1_722_493_000),
                updated_at: Some(1_722_493_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-17".into(),
                name: Some("[Base] You are operating inside the B".into()),
                preview: Some("Base agent context".into()),
                cwd: Some("~/Work/base".into()),
                created_at: Some(1_722_492_000),
                updated_at: Some(1_722_492_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-18".into(),
                name: Some("currently using krusty, and for sol".into()),
                preview: Some("Sol tooling".into()),
                cwd: Some("~/Work/sol".into()),
                created_at: Some(1_722_491_000),
                updated_at: Some(1_722_491_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-19".into(),
                name: Some("The Bullring Work".into()),
                preview: Some("Page tweaks".into()),
                cwd: None,
                created_at: Some(1_722_490_000),
                updated_at: Some(1_722_490_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-20".into(),
                name: Some("Krusty Work".into()),
                preview: Some("Core tasks".into()),
                cwd: Some("~/Work/krusty".into()),
                created_at: Some(1_722_489_000),
                updated_at: Some(1_722_489_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-21".into(),
                name: Some("https://github.com/blinklabs-io/hand".into()),
                preview: Some("External link thread".into()),
                cwd: None,
                created_at: Some(1_722_488_000),
                updated_at: Some(1_722_488_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(true),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-22".into(),
                name: Some("Sol-Dev Projects".into()),
                preview: Some("Workspace list".into()),
                cwd: Some("~/Work/sol-dev".into()),
                created_at: Some(1_722_487_000),
                updated_at: Some(1_722_487_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "demo-home-23".into(),
                name: Some("honeycomb.dev".into()),
                preview: Some("Site notes".into()),
                cwd: None,
                created_at: Some(1_722_486_000),
                updated_at: Some(1_722_486_100),
                model_provider: Some("fixture".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Codex,
            messages: vec![],
        },
        // Chat surface (mode=chat) — conversational threads, not coding agent work.
        DemoThread {
            summary: ThreadSummary {
                id: "chat-1".into(),
                name: Some("Morning check-in".into()),
                preview: Some("How’s your day going?".into()),
                cwd: None,
                created_at: Some(1_722_720_000),
                updated_at: Some(1_722_720_500),
                model_provider: Some("openai".into()),
                ephemeral: Some(false),
                is_pinned: Some(true),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Chat,
            messages: vec![
                DemoMessage::user("Hey Mitsuro — good morning. What’s a calm way to start the day?"),
                DemoMessage::assistant(
                    "Good morning. Three light steps: drink water, jot one intention for the day \
                     (not a whole plan), and clear two minutes of silence before opening inbox. \
                     Want a short checklist for focus or for rest?",
                ),
                DemoMessage::user("Focus, please — keep it under five bullets."),
                DemoMessage::assistant(
                    "Focus starter:\n\
                     1. Pick the single outcome that would make today a win\n\
                     2. Silence notifications for 45 minutes\n\
                     3. Start with the hardest 10-minute slice\n\
                     4. Stand up once mid-block\n\
                     5. Write a one-line note when you stop\n\
                     I’m here if you want to brainstorm that outcome.",
                ),
            ],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "chat-2".into(),
                name: Some("Travel ideas".into()),
                preview: Some("Quiet weekend getaway…".into()),
                cwd: None,
                created_at: Some(1_722_710_000),
                updated_at: Some(1_722_715_000),
                model_provider: Some("openai".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Chat,
            messages: vec![
                DemoMessage::user(
                    "Suggest a quiet weekend getaway within a few hours of a big city. \
                     Nature preferred, no packing list lectures.",
                ),
                DemoMessage::assistant(
                    "Think small and green: a lakeside cabin with a short hiking loop, \
                     a coastal town with empty morning beaches, or a vineyard B&B with \
                     long walks between villages. Tell me which climate you like — cool \
                     forest, ocean, or open hills — and I’ll narrow to three concrete ideas.",
                ),
                DemoMessage::user("Forest, cool weather."),
                DemoMessage::assistant(
                    "Forest shortlist: (1) a lodge beside a glacial lake with a 4-mile ridge trail, \
                     (2) a fire-tower cabin with stargazing decks, (3) a riverside inn where breakfast \
                     is the main event and trails start at the door. Pack layers; leave the laptop.",
                ),
            ],
        },
        DemoThread {
            summary: ThreadSummary {
                id: "chat-3".into(),
                name: Some("What is Mitsuro?".into()),
                preview: Some("Chat with Mitsuro · product intro".into()),
                cwd: None,
                created_at: Some(1_722_700_000),
                updated_at: Some(1_722_700_200),
                model_provider: Some("openai".into()),
                ephemeral: Some(false),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface: ThreadSurface::Chat,
            messages: vec![
                DemoMessage::user("What is Mitsuro in one sentence?"),
                DemoMessage::assistant(
                    "Mitsuro is a friendly desktop companion on Linux — Chat for conversation, \
                     Work for long goals, Codex when you need an agent — with Atlas, Terminal, \
                     and Settings in the same shell.",
                ),
            ],
        },
    ]
}

/// Offline Work goals (plan items) for the Work product mode.
pub fn demo_goals() -> Vec<DemoGoal> {
    vec![
        DemoGoal {
            id: "goal-1".into(),
            objective: "Ship multi-mode product shell (Chat · Work · Codex · Atlas)".into(),
            status: DemoGoalStatus::Active,
            thread_id: Some("demo-1".into()),
            updated_at: Some(1_722_730_000),
            plan_items: vec![
                DemoPlanItem {
                    id: "goal-1-a".into(),
                    title: "Expand ProductMode rail (Chat / Work / Codex / …)".into(),
                    done: true,
                },
                DemoPlanItem {
                    id: "goal-1-b".into(),
                    title: "Wire Chat surface + mode=chat demo threads".into(),
                    done: true,
                },
                DemoPlanItem {
                    id: "goal-1-c".into(),
                    title: "Work panel: create goal → plan tracker checklist".into(),
                    done: true,
                },
                DemoPlanItem {
                    id: "goal-1-d".into(),
                    title: "Wire thread/goal/set · get · clear via fixture".into(),
                    done: false,
                },
                DemoPlanItem {
                    id: "goal-1-e".into(),
                    title: "Preserve selection when switching modes".into(),
                    done: false,
                },
            ],
        },
        DemoGoal {
            id: "goal-2".into(),
            objective: "Polish Chat mode as a simple conversation surface".into(),
            status: DemoGoalStatus::Active,
            thread_id: Some("chat-1".into()),
            updated_at: Some(1_722_731_000),
            plan_items: vec![
                DemoPlanItem {
                    id: "goal-2-a".into(),
                    title: "Empty state: “Chat with Mitsuro”".into(),
                    done: true,
                },
                DemoPlanItem {
                    id: "goal-2-b".into(),
                    title: "Composer placeholder “Message…”".into(),
                    done: false,
                },
                DemoPlanItem {
                    id: "goal-2-c".into(),
                    title: "Simpler bubble layout for chat threads".into(),
                    done: false,
                },
            ],
        },
    ]
}

/// Default plan steps when the user creates a new Work goal (fixture).
pub fn new_goal_plan_items(goal_id: &str) -> Vec<DemoPlanItem> {
    vec![
        DemoPlanItem {
            id: format!("{goal_id}-a"),
            title: "Clarify objective and success criteria".into(),
            done: false,
        },
        DemoPlanItem {
            id: format!("{goal_id}-b"),
            title: "Break work into ordered plan steps".into(),
            done: false,
        },
        DemoPlanItem {
            id: format!("{goal_id}-c"),
            title: "Link a thread and track progress".into(),
            done: false,
        },
        DemoPlanItem {
            id: format!("{goal_id}-d"),
            title: "Review outcomes and mark complete".into(),
            done: false,
        },
    ]
}

pub fn meta_line(summary: &ThreadSummary) -> String {
    let mut parts = Vec::new();
    if let Some(cwd) = &summary.cwd {
        parts.push(shorten_path(cwd));
    }
    if let Some(preview) = &summary.preview {
        if summary.cwd.is_none() || !preview.is_empty() {
            parts.push(preview.clone());
        }
    }
    if parts.is_empty() {
        "Local thread".into()
    } else {
        parts.join(" · ")
    }
}

fn shorten_path(path: &str) -> String {
    if path.len() <= 36 {
        return path.to_string();
    }
    let tail: String = path
        .chars()
        .rev()
        .take(32)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("…{tail}")
}
