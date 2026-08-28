use super::*;

fn context(seed: &str) -> WorkerIntroductionPresentationContext<'_> {
    WorkerIntroductionPresentationContext::new("Navigator", "navigator", seed)
}

fn assert_safe_render(rendered: &str, expected_questions: usize) {
    let lower = rendered.to_lowercase();
    assert_eq!(
        rendered.matches('?').count(),
        expected_questions,
        "unexpected question count in {rendered:?}"
    );
    assert!(!rendered.contains('\n'));
    assert!(!rendered.contains("  "));
    assert!(rendered.len() <= 512, "render is not concise: {rendered:?}");
    for forbidden in [
        "sentient",
        "alive",
        "born",
        "birth",
        "i feel",
        "feelings",
        "worker bee",
        "bee persona",
        "buzz",
        "saved",
        "remembered",
        "recorded",
        "applied",
    ] {
        assert!(
            !lower.contains(forbidden),
            "render contains forbidden claim/persona token {forbidden:?}: {rendered:?}"
        );
    }
}

fn assert_complete_opening_curiosity(rendered: &str) {
    let lower = rendered.to_lowercase();
    for (axis, terms) in [
        ("identity", &["role", "who"] as &[&str]),
        ("purpose", &["purpose", "help", "outcome"]),
        ("working style", &["prefer to work", "working style"]),
        ("boundaries", &["boundar"]),
        ("tools", &["tools"]),
        ("memory", &["memory"]),
        ("cadence", &["cadence", "initiative"]),
    ] {
        assert!(
            terms.iter().any(|term| lower.contains(term)),
            "opening omits {axis} curiosity: {rendered:?}"
        );
    }
}

#[test]
fn strict_intent_parsers_accept_only_v1_enum_json() {
    let opening = parse_worker_introduction_opening_intent(
        r#"{"schema_version":1,"tone":"thoughtful","question_topic":"boundaries"}"#,
    )
    .unwrap();
    assert_eq!(opening.schema_version, 1);
    assert_eq!(opening.tone, WorkerIntroductionOpeningTone::Thoughtful);
    assert_eq!(
        opening.question_topic,
        WorkerIntroductionQuestionTopic::Boundaries
    );
    assert_eq!(
        parse_worker_introduction_opening_intent(&serde_json::to_string(&opening).unwrap())
            .unwrap(),
        opening
    );

    let reply = parse_worker_introduction_onboarding_reply_intent(
        r#"{"schema_version":1,"acknowledgement":"collaborative","follow_up_topic":"tools_and_memory_expectations"}"#,
    )
    .unwrap();
    assert_eq!(
        reply.acknowledgement,
        WorkerIntroductionAcknowledgement::Collaborative
    );
    assert_eq!(
        reply.follow_up_topic,
        Some(WorkerIntroductionQuestionTopic::ToolsAndMemoryExpectations)
    );
    assert_eq!(
        parse_worker_introduction_onboarding_reply_intent(
            r#"{"schema_version":1,"acknowledgement":"neutral"}"#
        )
        .unwrap()
        .follow_up_topic,
        None
    );
    assert_eq!(
        parse_worker_introduction_onboarding_reply_intent(
            r#"{"schema_version":1,"acknowledgement":"neutral","follow_up_topic":null}"#
        )
        .unwrap()
        .follow_up_topic,
        None
    );
}

#[test]
fn strict_intent_parsers_reject_malformed_adversarial_and_oversized_payloads() {
    let invalid_openings = [
        "",
        "not json",
        "```json\n{\"schema_version\":1,\"tone\":\"warm\",\"question_topic\":\"identity\"}\n```",
        "{\"schema_version\":1,\"tone\":\"warm\",\"question_topic\":\"identity\"} trailing",
        "{\"schema_version\":2,\"tone\":\"warm\",\"question_topic\":\"identity\"}",
        "{\"schema_version\":1,\"tone\":\"friendly\",\"question_topic\":\"identity\"}",
        "{\"schema_version\":1,\"tone\":\"warm\",\"question_topic\":\"identity\",\"message\":\"I am alive\"}",
        "{\"schema_version\":1,\"tone\":\"warm\",\"tone\":\"direct\",\"question_topic\":\"identity\"}",
        "{\"schema_version\":1,\"tone\":\"warm\"}",
        "[1,2,3]",
    ];
    for raw in invalid_openings {
        assert!(
            parse_worker_introduction_opening_intent(raw).is_err(),
            "opening payload should fail: {raw:?}"
        );
    }

    let invalid_replies = [
        "",
        "Here is the JSON: {\"schema_version\":1,\"acknowledgement\":\"neutral\"}",
        "```{\"schema_version\":1,\"acknowledgement\":\"neutral\"}```",
        "{\"schema_version\":0,\"acknowledgement\":\"neutral\"}",
        "{\"schema_version\":1,\"acknowledgement\":\"excited\"}",
        "{\"schema_version\":1,\"acknowledgement\":\"neutral\",\"follow_up_topic\":[\"identity\",\"boundaries\"]}",
        "{\"schema_version\":1,\"acknowledgement\":\"neutral\",\"follow_up_topic\":\"identity\",\"follow_up_topic\":\"boundaries\"}",
        "{\"schema_version\":1,\"acknowledgement\":\"neutral\",\"visible_text\":\"I remembered that\"}",
        "{\"schema_version\":1}",
    ];
    for raw in invalid_replies {
        assert!(
            parse_worker_introduction_onboarding_reply_intent(raw).is_err(),
            "reply payload should fail: {raw:?}"
        );
    }

    let oversized = format!(
        "{}{}",
        r#"{"schema_version":1,"tone":"warm","question_topic":"identity"}"#,
        " ".repeat(MAX_INTENT_RESPONSE_BYTES)
    );
    assert!(parse_worker_introduction_opening_intent(&oversized).is_err());
}

#[test]
fn every_opening_template_combination_is_safe_and_asks_exactly_one_question() {
    for tone in WorkerIntroductionOpeningTone::ALL {
        for question_topic in WorkerIntroductionQuestionTopic::ALL {
            let intent = WorkerIntroductionOpeningIntentV1 {
                schema_version: WORKER_INTRODUCTION_PRESENTATION_VERSION,
                tone,
                question_topic,
            };
            for lead_index in 0..opening_leads(tone).len() {
                for question_index in 0..opening_questions(question_topic).len() {
                    let rendered = render_opening_with_variants(
                        &intent,
                        context("exhaustive-opening"),
                        lead_index,
                        question_index,
                    );
                    assert_safe_render(&rendered, 1);
                    assert_complete_opening_curiosity(&rendered);
                }
            }
            for question in opening_questions(question_topic) {
                let lower = question.to_lowercase();
                assert!(
                    lower.contains("who")
                        || lower.contains("role")
                        || lower.contains("contribute")
                        || lower.contains("fit into"),
                    "opening question omits identity/role axis: {question:?}"
                );
                assert!(
                    lower.contains("help")
                        || lower.contains("outcome")
                        || lower.contains("purpose")
                        || lower.contains("focus"),
                    "opening question omits purpose/help axis: {question:?}"
                );
            }
            let rendered =
                render_worker_introduction_opening(&intent, context("public-opening")).unwrap();
            assert_safe_render(&rendered, 1);
            assert_complete_opening_curiosity(&rendered);
        }
    }
}

#[test]
fn every_onboarding_template_combination_has_zero_or_one_safe_question() {
    for acknowledgement in WorkerIntroductionAcknowledgement::ALL {
        let without_question = WorkerIntroductionOnboardingReplyIntentV1 {
            schema_version: WORKER_INTRODUCTION_PRESENTATION_VERSION,
            acknowledgement,
            follow_up_topic: None,
        };
        for acknowledgement_index in 0..acknowledgement_templates(acknowledgement).len() {
            assert_safe_render(
                &render_onboarding_with_variants(&without_question, acknowledgement_index, None),
                0,
            );
        }
        assert_safe_render(
            &render_worker_introduction_onboarding_reply(
                &without_question,
                context("public-no-question"),
            )
            .unwrap(),
            0,
        );

        for topic in WorkerIntroductionQuestionTopic::ALL {
            let with_question = WorkerIntroductionOnboardingReplyIntentV1 {
                schema_version: WORKER_INTRODUCTION_PRESENTATION_VERSION,
                acknowledgement,
                follow_up_topic: Some(topic),
            };
            for acknowledgement_index in 0..acknowledgement_templates(acknowledgement).len() {
                for question_index in 0..follow_up_questions(topic).len() {
                    assert_safe_render(
                        &render_onboarding_with_variants(
                            &with_question,
                            acknowledgement_index,
                            Some(question_index),
                        ),
                        1,
                    );
                }
            }
            assert_safe_render(
                &render_worker_introduction_onboarding_reply(
                    &with_question,
                    context("public-one-question"),
                )
                .unwrap(),
                1,
            );
        }
    }
}

#[test]
fn renderer_sanitizes_and_bounds_names_without_changing_question_counts() {
    let intent = WorkerIntroductionOpeningIntentV1 {
        schema_version: 1,
        tone: WorkerIntroductionOpeningTone::Warm,
        question_topic: WorkerIntroductionQuestionTopic::Identity,
    };
    let rendered = render_worker_introduction_opening(
        &intent,
        WorkerIntroductionPresentationContext::new(
            "  ?{{Navigator}}\n<script>  ",
            "safe-worker",
            "name-safety",
        ),
    )
    .unwrap();
    assert!(!rendered.contains('?') || rendered.ends_with('?'));
    assert!(!rendered
        .chars()
        .any(|character| matches!(character, '{' | '}' | '<' | '>' | '\n')));
    assert_safe_render(&rendered, 1);

    let long_name = "界".repeat(200);
    let bounded = render_worker_introduction_opening(
        &intent,
        WorkerIntroductionPresentationContext::new(&long_name, "safe-worker", "bounded-name"),
    )
    .unwrap();
    assert_safe_render(&bounded, 1);
    assert!(bounded.len() < 512);

    let fallback_name = render_worker_introduction_opening(
        &intent,
        WorkerIntroductionPresentationContext::new("???", "safe-worker", "slug-fallback"),
    )
    .unwrap();
    assert!(fallback_name.contains("safe worker"));
    assert_safe_render(&fallback_name, 1);

    let claim_like_name = render_worker_introduction_opening(
        &intent,
        WorkerIntroductionPresentationContext::new(
            "I saved and remembered everything?",
            "safe-worker",
            "claim-name-fallback",
        ),
    )
    .unwrap();
    assert!(claim_like_name.contains("safe worker"));
    assert_safe_render(&claim_like_name, 1);
}

#[test]
fn fallbacks_are_explicit_deterministic_v1_and_safely_renderable() {
    let opening_a = fallback_worker_introduction_opening_intent("run-123");
    let opening_b = fallback_worker_introduction_opening_intent("run-123");
    assert_eq!(opening_a, opening_b);
    assert_eq!(opening_a.schema_version, 1);
    assert_eq!(opening_a.tone, WorkerIntroductionOpeningTone::Thoughtful);
    assert_eq!(
        opening_a.question_topic,
        WorkerIntroductionQuestionTopic::Identity
    );
    let fallback_opening =
        render_worker_introduction_opening(&opening_a, context("run-123")).unwrap();
    assert!(fallback_opening.contains("without assumptions"));
    assert_safe_render(&fallback_opening, 1);

    let reply_with_question =
        fallback_worker_introduction_onboarding_reply_intent("message-42", true);
    let repeated = fallback_worker_introduction_onboarding_reply_intent("message-42", true);
    assert_eq!(reply_with_question, repeated);
    assert!(reply_with_question.follow_up_topic.is_some());
    assert_safe_render(
        &render_worker_introduction_onboarding_reply(&reply_with_question, context("message-42"))
            .unwrap(),
        1,
    );

    let reply_without_question =
        fallback_worker_introduction_onboarding_reply_intent("message-42", false);
    assert!(reply_without_question.follow_up_topic.is_none());
    assert_safe_render(
        &render_worker_introduction_onboarding_reply(
            &reply_without_question,
            context("message-42"),
        )
        .unwrap(),
        0,
    );
}

#[test]
fn response_instructions_enumerate_the_closed_contract_without_visible_prose_fields() {
    let opening = worker_introduction_opening_intent_instructions();
    let onboarding = worker_introduction_onboarding_reply_intent_instructions();
    for tone in WorkerIntroductionOpeningTone::ALL {
        assert!(opening.contains(tone.as_str()));
    }
    for acknowledgement in WorkerIntroductionAcknowledgement::ALL {
        assert!(onboarding.contains(acknowledgement.as_str()));
    }
    for topic in WorkerIntroductionQuestionTopic::ALL {
        assert!(opening.contains(topic.as_str()));
        assert!(onboarding.contains(topic.as_str()));
    }
    assert!(opening.contains("no markdown"));
    assert!(onboarding.contains("no markdown"));
    assert!(!opening.contains("\"message\""));
    assert!(!onboarding.contains("\"message\""));
}

#[test]
fn renderer_rejects_manually_constructed_future_schema_versions() {
    let opening = WorkerIntroductionOpeningIntentV1 {
        schema_version: 2,
        tone: WorkerIntroductionOpeningTone::Warm,
        question_topic: WorkerIntroductionQuestionTopic::Identity,
    };
    assert!(render_worker_introduction_opening(&opening, context("future")).is_err());

    let reply = WorkerIntroductionOnboardingReplyIntentV1 {
        schema_version: 2,
        acknowledgement: WorkerIntroductionAcknowledgement::Neutral,
        follow_up_topic: None,
    };
    assert!(render_worker_introduction_onboarding_reply(&reply, context("future")).is_err());
}
