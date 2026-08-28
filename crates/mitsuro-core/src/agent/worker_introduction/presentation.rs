//! Typed, provider-independent presentation for Worker Introductions.
//!
//! Providers select only bounded enums. Trusted core code owns every visible
//! sentence, so an opening or onboarding reply cannot smuggle model-authored
//! biography, persistence claims, or persona text into the conversation.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const WORKER_INTRODUCTION_PRESENTATION_VERSION: u8 = 1;

const MAX_INTENT_RESPONSE_BYTES: usize = 512;
const MAX_RENDERED_NAME_BYTES: usize = 80;
const MAX_SEED_BYTES: usize = 512;

const OPENING_INTENT_INSTRUCTIONS: &str = r#"Select the presentation intent for a new Mitsuro Hive Worker.

Return exactly one JSON object and no markdown, code fence, explanation, or visible reply text:
{"schema_version":1,"tone":"warm","question_topic":"purpose_and_help"}

schema_version must be 1.
tone must be exactly one of: warm, direct, thoughtful, upbeat.
question_topic must be exactly one of: identity, purpose_and_help, working_style, boundaries, tools_and_memory_expectations, cadence_and_initiative.
question_topic selects the first question's emphasis only. Trusted rendering will always express curiosity about the Worker's role and purpose, working style, boundaries, tools and memory, and cadence or initiative before asking one natural first question.
Do not add fields. Do not write the Worker's message, name, biography, feelings, capabilities, memory claims, or any other prose."#;

const ONBOARDING_REPLY_INTENT_INSTRUCTIONS: &str = r#"Select the presentation intent for a Mitsuro Hive Worker's onboarding reply.

Return exactly one JSON object and no markdown, code fence, explanation, or visible reply text:
{"schema_version":1,"acknowledgement":"appreciative","follow_up_topic":"working_style"}

schema_version must be 1.
acknowledgement must be exactly one of: appreciative, focused, collaborative, neutral.
follow_up_topic must be null, omitted, or exactly one of: identity, purpose_and_help, working_style, boundaries, tools_and_memory_expectations, cadence_and_initiative.
Do not add fields. Do not write the Worker's message, quote the user, claim anything was saved or remembered, or return any other prose."#;

/// The bounded tone selected for a server-rendered opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionOpeningTone {
    Warm,
    Direct,
    Thoughtful,
    Upbeat,
}

impl WorkerIntroductionOpeningTone {
    pub const ALL: [Self; 4] = [Self::Warm, Self::Direct, Self::Thoughtful, Self::Upbeat];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warm => "warm",
            Self::Direct => "direct",
            Self::Thoughtful => "thoughtful",
            Self::Upbeat => "upbeat",
        }
    }
}

/// The only questions trusted presentation code may ask during Introduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionQuestionTopic {
    Identity,
    PurposeAndHelp,
    WorkingStyle,
    Boundaries,
    ToolsAndMemoryExpectations,
    CadenceAndInitiative,
}

impl WorkerIntroductionQuestionTopic {
    pub const ALL: [Self; 6] = [
        Self::Identity,
        Self::PurposeAndHelp,
        Self::WorkingStyle,
        Self::Boundaries,
        Self::ToolsAndMemoryExpectations,
        Self::CadenceAndInitiative,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::PurposeAndHelp => "purpose_and_help",
            Self::WorkingStyle => "working_style",
            Self::Boundaries => "boundaries",
            Self::ToolsAndMemoryExpectations => "tools_and_memory_expectations",
            Self::CadenceAndInitiative => "cadence_and_initiative",
        }
    }
}

/// Strict provider output for the first visible Worker message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionOpeningIntentV1 {
    pub schema_version: u8,
    pub tone: WorkerIntroductionOpeningTone,
    pub question_topic: WorkerIntroductionQuestionTopic,
}

/// The bounded acknowledgement selected for an onboarding reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIntroductionAcknowledgement {
    Appreciative,
    Focused,
    Collaborative,
    Neutral,
}

impl WorkerIntroductionAcknowledgement {
    pub const ALL: [Self; 4] = [
        Self::Appreciative,
        Self::Focused,
        Self::Collaborative,
        Self::Neutral,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Appreciative => "appreciative",
            Self::Focused => "focused",
            Self::Collaborative => "collaborative",
            Self::Neutral => "neutral",
        }
    }
}

/// Strict provider output after one canonical user onboarding message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerIntroductionOnboardingReplyIntentV1 {
    pub schema_version: u8,
    pub acknowledgement: WorkerIntroductionAcknowledgement,
    #[serde(default)]
    pub follow_up_topic: Option<WorkerIntroductionQuestionTopic>,
}

/// Trusted inputs used only by the deterministic renderer.
///
/// `display_name` and `slug` must come from the persisted Worker, never the
/// provider response. The renderer still normalizes and bounds the name so a
/// legacy malformed value cannot add punctuation or template syntax.
#[derive(Debug, Clone, Copy)]
pub struct WorkerIntroductionPresentationContext<'a> {
    pub display_name: &'a str,
    pub slug: &'a str,
    pub deterministic_seed: &'a str,
}

impl<'a> WorkerIntroductionPresentationContext<'a> {
    pub const fn new(display_name: &'a str, slug: &'a str, deterministic_seed: &'a str) -> Self {
        Self {
            display_name,
            slug,
            deterministic_seed,
        }
    }
}

/// Strict response instructions for the opening intent provider call.
pub const fn worker_introduction_opening_intent_instructions() -> &'static str {
    OPENING_INTENT_INSTRUCTIONS
}

/// Strict response instructions for the post-user-message intent call.
pub const fn worker_introduction_onboarding_reply_intent_instructions() -> &'static str {
    ONBOARDING_REPLY_INTENT_INSTRUCTIONS
}

/// Parse exact JSON only. Markdown fences and surrounding prose are rejected.
pub fn parse_worker_introduction_opening_intent(
    raw: &str,
) -> Result<WorkerIntroductionOpeningIntentV1> {
    let intent: WorkerIntroductionOpeningIntentV1 = parse_strict_intent(raw, "opening")?;
    ensure!(
        intent.schema_version == WORKER_INTRODUCTION_PRESENTATION_VERSION,
        "Worker Introduction opening intent has an unsupported schema version"
    );
    Ok(intent)
}

/// Parse exact JSON only. Markdown fences and surrounding prose are rejected.
pub fn parse_worker_introduction_onboarding_reply_intent(
    raw: &str,
) -> Result<WorkerIntroductionOnboardingReplyIntentV1> {
    let intent: WorkerIntroductionOnboardingReplyIntentV1 =
        parse_strict_intent(raw, "onboarding reply")?;
    ensure!(
        intent.schema_version == WORKER_INTRODUCTION_PRESENTATION_VERSION,
        "Worker Introduction onboarding reply intent has an unsupported schema version"
    );
    Ok(intent)
}

/// Explicit, deterministic fallback after a bounded opening-intent failure.
///
/// Parsing never invokes this implicitly; the caller decides when its retry
/// budget is exhausted and records that fallback decision at the runtime
/// boundary.
pub fn fallback_worker_introduction_opening_intent(
    _deterministic_seed: &str,
) -> WorkerIntroductionOpeningIntentV1 {
    WorkerIntroductionOpeningIntentV1 {
        schema_version: WORKER_INTRODUCTION_PRESENTATION_VERSION,
        // A failed provider call should still produce the strongest version
        // of the user's desired opening: named, assumption-free, and curious
        // about both identity and purpose.
        tone: WorkerIntroductionOpeningTone::Thoughtful,
        question_topic: WorkerIntroductionQuestionTopic::Identity,
    }
}

/// Explicit, deterministic fallback after a bounded onboarding-intent failure.
///
/// `include_follow_up` is an authoritative lifecycle choice. Passing `false`
/// yields an acknowledgement with no question; passing `true` keeps context
/// gathering moving with exactly one safe question.
pub fn fallback_worker_introduction_onboarding_reply_intent(
    deterministic_seed: &str,
    include_follow_up: bool,
) -> WorkerIntroductionOnboardingReplyIntentV1 {
    WorkerIntroductionOnboardingReplyIntentV1 {
        schema_version: WORKER_INTRODUCTION_PRESENTATION_VERSION,
        acknowledgement: WorkerIntroductionAcknowledgement::ALL[deterministic_index(
            deterministic_seed,
            &["fallback", "onboarding", "acknowledgement"],
            WorkerIntroductionAcknowledgement::ALL.len(),
        )],
        follow_up_topic: include_follow_up.then(|| {
            WorkerIntroductionQuestionTopic::ALL[deterministic_index(
                deterministic_seed,
                &["fallback", "onboarding", "topic"],
                WorkerIntroductionQuestionTopic::ALL.len(),
            )]
        }),
    }
}

/// Render the first visible Worker message. Every path contains one question.
pub fn render_worker_introduction_opening(
    intent: &WorkerIntroductionOpeningIntentV1,
    context: WorkerIntroductionPresentationContext<'_>,
) -> Result<String> {
    validate_version(intent.schema_version, "opening")?;
    let leads = opening_leads(intent.tone);
    let questions = opening_questions(intent.question_topic);
    Ok(render_opening_with_variants(
        intent,
        context,
        deterministic_index(
            context.deterministic_seed,
            &["render", "opening", "lead", intent.tone.as_str()],
            leads.len(),
        ),
        deterministic_index(
            context.deterministic_seed,
            &[
                "render",
                "opening",
                "question",
                intent.question_topic.as_str(),
            ],
            questions.len(),
        ),
    ))
}

/// Render a visible onboarding reply. `None` produces no question and `Some`
/// produces exactly one question.
pub fn render_worker_introduction_onboarding_reply(
    intent: &WorkerIntroductionOnboardingReplyIntentV1,
    context: WorkerIntroductionPresentationContext<'_>,
) -> Result<String> {
    validate_version(intent.schema_version, "onboarding reply")?;
    let acknowledgements = acknowledgement_templates(intent.acknowledgement);
    let acknowledgement_index = deterministic_index(
        context.deterministic_seed,
        &[
            "render",
            "onboarding",
            "acknowledgement",
            intent.acknowledgement.as_str(),
        ],
        acknowledgements.len(),
    );
    let question_index = intent.follow_up_topic.map(|topic| {
        deterministic_index(
            context.deterministic_seed,
            &["render", "onboarding", "question", topic.as_str()],
            follow_up_questions(topic).len(),
        )
    });
    Ok(render_onboarding_with_variants(
        intent,
        acknowledgement_index,
        question_index,
    ))
}

fn parse_strict_intent<T>(raw: &str, label: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    ensure!(
        raw.len() <= MAX_INTENT_RESPONSE_BYTES,
        "Worker Introduction {label} intent exceeds the byte limit"
    );
    ensure!(
        !raw.trim().is_empty(),
        "Worker Introduction {label} intent is empty"
    );
    serde_json::from_str(raw.trim())
        .with_context(|| format!("Worker Introduction {label} intent is not strict JSON"))
}

fn validate_version(version: u8, label: &str) -> Result<()> {
    ensure!(
        version == WORKER_INTRODUCTION_PRESENTATION_VERSION,
        "Worker Introduction {label} intent has an unsupported schema version"
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct NamedLead {
    before_name: &'static str,
    after_name: &'static str,
}

const WARM_LEADS: [NamedLead; 2] = [
    NamedLead {
        before_name: "Hi — I’m ",
        after_name: ". Let’s shape how I can help.",
    },
    NamedLead {
        before_name: "Welcome — I’m ",
        after_name: ". We can define a useful way to work together.",
    },
];
const DIRECT_LEADS: [NamedLead; 2] = [
    NamedLead {
        before_name: "I’m ",
        after_name: ". Let’s define the role clearly.",
    },
    NamedLead {
        before_name: "I’m ",
        after_name: ". A clear starting point will keep the work focused.",
    },
];
const THOUGHTFUL_LEADS: [NamedLead; 2] = [
    NamedLead {
        before_name: "I’m ",
        after_name: ". I’ll start without assumptions about the role.",
    },
    NamedLead {
        before_name: "I’m ",
        after_name: ". I’ll begin without assumptions and follow your direction.",
    },
];
const UPBEAT_LEADS: [NamedLead; 2] = [
    NamedLead {
        before_name: "Hi, I’m ",
        after_name: ". Let’s build a useful way of working together.",
    },
    NamedLead {
        before_name: "I’m ",
        after_name: ". Let’s set a useful direction together.",
    },
];

/// The opening names the complete onboarding scope without turning the first
/// message into a questionnaire. The selected topic below still owns the one
/// natural first question; later turns gather and confirm the remaining
/// context one step at a time.
const OPENING_CURIOSITY_SCOPE: &str = "I’m curious about the role and purpose you have in mind, how you prefer to work, the boundaries I should respect, how tools and memory should fit, and the cadence or initiative that feels right.";

const OPENING_IDENTITY_QUESTIONS: [&str; 3] = [
    "Who should I be in your work, and what should I help you accomplish?",
    "What role should define me, and where would that role help most?",
    "How should I fit into your team, and what purpose should guide my work?",
];
const OPENING_PURPOSE_QUESTIONS: [&str; 3] = [
    "What role should I take, and what would you most like me to help with?",
    "Who should I be for you, and which outcomes should I help you pursue?",
    "How should I contribute, and what purpose should guide that role?",
];
const OPENING_WORKING_STYLE_QUESTIONS: [&str; 3] = [
    "What role should I take, what should I help with, and how should we work together?",
    "Who should I be in your work, what outcomes matter, and which collaboration style suits you?",
    "How should I contribute, what should I focus on, and what working style would be most useful?",
];
const OPENING_BOUNDARY_QUESTIONS: [&str; 3] = [
    "Who should I be, what should I help with, and which boundaries matter from the start?",
    "What role should I take, what outcomes should I support, and what should stay under your control?",
    "How should I contribute, where should I focus, and what should I avoid without approval?",
];
const OPENING_TOOLS_AND_MEMORY_QUESTIONS: [&str; 3] = [
    "Who should I be, what should I help with, and how should tools or memory fit into that work?",
    "What role should I take, which outcomes matter, and what context should carry forward?",
    "How should I contribute, what should I focus on, and when should tools or memory be useful?",
];
const OPENING_CADENCE_QUESTIONS: [&str; 3] = [
    "Who should I be, what should I help with, and how proactive should I be?",
    "What role should I take, which outcomes matter, and what cadence should I follow?",
    "How should I contribute, where should I focus, and when should I take initiative?",
];

const FOLLOW_UP_IDENTITY_QUESTIONS: [&str; 3] = [
    "What role should I take when working with you?",
    "How would you describe the role you want this Worker to take?",
    "What should define this Worker’s identity in your team?",
];
const FOLLOW_UP_PURPOSE_QUESTIONS: [&str; 3] = [
    "What would you most like me to help with?",
    "Which outcomes should I focus on first?",
    "What purpose should guide my work with you?",
];
const FOLLOW_UP_WORKING_STYLE_QUESTIONS: [&str; 3] = [
    "How do you prefer us to work through problems together?",
    "What working style makes collaboration most useful for you?",
    "Should I be concise, exploratory, proactive, or something else?",
];
const FOLLOW_UP_BOUNDARY_QUESTIONS: [&str; 3] = [
    "What boundaries should I respect while helping?",
    "Are there decisions or actions I should always leave to you?",
    "What should I avoid doing without your approval?",
];
const FOLLOW_UP_TOOLS_AND_MEMORY_QUESTIONS: [&str; 3] = [
    "How should I use tools and long-term memory in our work?",
    "What context should carry forward, and what should stay temporary?",
    "When should I use tools, and what context should carry forward?",
];
const FOLLOW_UP_CADENCE_QUESTIONS: [&str; 3] = [
    "How proactive should I be between direct requests?",
    "What cadence should I follow for check-ins or ongoing work?",
    "When should I take initiative, and when should I wait for you?",
];

const APPRECIATIVE_ACKNOWLEDGEMENTS: [&str; 2] = [
    "Thanks for the context.",
    "Thanks — that gives us a useful starting point.",
];
const FOCUSED_ACKNOWLEDGEMENTS: [&str; 2] = [
    "Understood. We can narrow the role from here.",
    "That sets a clear direction for the next step.",
];
const COLLABORATIVE_ACKNOWLEDGEMENTS: [&str; 2] = [
    "Good starting point. We can refine it together.",
    "That gives us something concrete to shape together.",
];
const NEUTRAL_ACKNOWLEDGEMENTS: [&str; 2] = [
    "Got it. We can continue from there.",
    "Okay. Let’s take the next step.",
];

fn opening_leads(tone: WorkerIntroductionOpeningTone) -> &'static [NamedLead] {
    match tone {
        WorkerIntroductionOpeningTone::Warm => &WARM_LEADS,
        WorkerIntroductionOpeningTone::Direct => &DIRECT_LEADS,
        WorkerIntroductionOpeningTone::Thoughtful => &THOUGHTFUL_LEADS,
        WorkerIntroductionOpeningTone::Upbeat => &UPBEAT_LEADS,
    }
}

fn opening_questions(topic: WorkerIntroductionQuestionTopic) -> &'static [&'static str] {
    match topic {
        WorkerIntroductionQuestionTopic::Identity => &OPENING_IDENTITY_QUESTIONS,
        WorkerIntroductionQuestionTopic::PurposeAndHelp => &OPENING_PURPOSE_QUESTIONS,
        WorkerIntroductionQuestionTopic::WorkingStyle => &OPENING_WORKING_STYLE_QUESTIONS,
        WorkerIntroductionQuestionTopic::Boundaries => &OPENING_BOUNDARY_QUESTIONS,
        WorkerIntroductionQuestionTopic::ToolsAndMemoryExpectations => {
            &OPENING_TOOLS_AND_MEMORY_QUESTIONS
        }
        WorkerIntroductionQuestionTopic::CadenceAndInitiative => &OPENING_CADENCE_QUESTIONS,
    }
}

fn follow_up_questions(topic: WorkerIntroductionQuestionTopic) -> &'static [&'static str] {
    match topic {
        WorkerIntroductionQuestionTopic::Identity => &FOLLOW_UP_IDENTITY_QUESTIONS,
        WorkerIntroductionQuestionTopic::PurposeAndHelp => &FOLLOW_UP_PURPOSE_QUESTIONS,
        WorkerIntroductionQuestionTopic::WorkingStyle => &FOLLOW_UP_WORKING_STYLE_QUESTIONS,
        WorkerIntroductionQuestionTopic::Boundaries => &FOLLOW_UP_BOUNDARY_QUESTIONS,
        WorkerIntroductionQuestionTopic::ToolsAndMemoryExpectations => {
            &FOLLOW_UP_TOOLS_AND_MEMORY_QUESTIONS
        }
        WorkerIntroductionQuestionTopic::CadenceAndInitiative => &FOLLOW_UP_CADENCE_QUESTIONS,
    }
}

fn acknowledgement_templates(
    acknowledgement: WorkerIntroductionAcknowledgement,
) -> &'static [&'static str] {
    match acknowledgement {
        WorkerIntroductionAcknowledgement::Appreciative => &APPRECIATIVE_ACKNOWLEDGEMENTS,
        WorkerIntroductionAcknowledgement::Focused => &FOCUSED_ACKNOWLEDGEMENTS,
        WorkerIntroductionAcknowledgement::Collaborative => &COLLABORATIVE_ACKNOWLEDGEMENTS,
        WorkerIntroductionAcknowledgement::Neutral => &NEUTRAL_ACKNOWLEDGEMENTS,
    }
}

fn render_opening_with_variants(
    intent: &WorkerIntroductionOpeningIntentV1,
    context: WorkerIntroductionPresentationContext<'_>,
    lead_index: usize,
    question_index: usize,
) -> String {
    let leads = opening_leads(intent.tone);
    let questions = opening_questions(intent.question_topic);
    let lead = leads[lead_index % leads.len()];
    let question = questions[question_index % questions.len()];
    let name = sanitized_worker_name(context.display_name, context.slug);
    let mut rendered = String::with_capacity(
        lead.before_name.len()
            + name.len()
            + lead.after_name.len()
            + OPENING_CURIOSITY_SCOPE.len()
            + question.len()
            + 2,
    );
    rendered.push_str(lead.before_name);
    rendered.push_str(&name);
    rendered.push_str(lead.after_name);
    rendered.push(' ');
    rendered.push_str(OPENING_CURIOSITY_SCOPE);
    rendered.push(' ');
    rendered.push_str(question);
    rendered
}

fn render_onboarding_with_variants(
    intent: &WorkerIntroductionOnboardingReplyIntentV1,
    acknowledgement_index: usize,
    question_index: Option<usize>,
) -> String {
    let acknowledgements = acknowledgement_templates(intent.acknowledgement);
    let acknowledgement = acknowledgements[acknowledgement_index % acknowledgements.len()];
    let Some(topic) = intent.follow_up_topic else {
        return acknowledgement.to_string();
    };
    let questions = follow_up_questions(topic);
    let question = questions[question_index.unwrap_or(0) % questions.len()];
    format!("{acknowledgement} {question}")
}

fn sanitized_worker_name(display_name: &str, slug: &str) -> String {
    let display_name = sanitized_name_candidate(display_name, false);
    if !display_name.is_empty() && !contains_reserved_presentation_claim(&display_name) {
        return display_name;
    }
    let slug = sanitized_name_candidate(slug, true);
    if slug.is_empty() || contains_reserved_presentation_claim(&slug) {
        "Worker".to_string()
    } else {
        slug
    }
}

fn contains_reserved_presentation_claim(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            matches!(
                word,
                "sentient"
                    | "alive"
                    | "born"
                    | "birth"
                    | "feel"
                    | "feelings"
                    | "bee"
                    | "buzz"
                    | "saved"
                    | "remembered"
                    | "recorded"
                    | "applied"
            )
        })
}

fn sanitized_name_candidate(raw: &str, separators_as_spaces: bool) -> String {
    let mut rendered = String::with_capacity(raw.len().min(MAX_RENDERED_NAME_BYTES));
    let mut pending_space = false;
    for character in raw.trim().chars() {
        let is_separator =
            character.is_whitespace() || (separators_as_spaces && matches!(character, '-' | '_'));
        if is_separator {
            pending_space = !rendered.is_empty();
            continue;
        }
        if !(character.is_alphanumeric() || matches!(character, '-' | '_' | '\'' | '.')) {
            continue;
        }
        let additional = character.len_utf8() + if pending_space { 1 } else { 0 };
        if rendered.len() + additional > MAX_RENDERED_NAME_BYTES {
            break;
        }
        if pending_space {
            rendered.push(' ');
            pending_space = false;
        }
        rendered.push(character);
    }
    rendered
        .trim_matches(|character: char| {
            character.is_whitespace() || matches!(character, '-' | '_' | '\'' | '.')
        })
        .to_string()
}

fn deterministic_index(seed: &str, domain: &[&str], len: usize) -> usize {
    debug_assert!(len > 0);
    let mut digest = Sha256::new();
    for component in domain {
        digest.update(component.as_bytes());
        digest.update([0]);
    }
    digest.update(&seed.as_bytes()[..seed.len().min(MAX_SEED_BYTES)]);
    let bytes = digest.finalize();
    let value = u64::from_be_bytes(
        bytes[..8]
            .try_into()
            .expect("SHA-256 prefix is eight bytes"),
    );
    value as usize % len
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
