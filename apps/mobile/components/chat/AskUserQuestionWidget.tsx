import { useMemo, useState } from "react";
import { View, Text, Pressable, TextInput, StyleSheet } from "react-native";
import { HelpCircle, Check, Send } from "lucide-react-native";
import { useThemeContext } from "../../hooks/useTheme";
import type { ToolCall } from "@mitsuro/api";

interface QuestionOption {
  label: string;
  description?: string;
}

interface Question {
  question: string;
  header: string;
  options: QuestionOption[];
  multiSelect?: boolean;
}

interface AskUserQuestionWidgetProps {
  toolCall: ToolCall;
  isSubmitting?: boolean;
  onSubmit: (result: string) => void | Promise<void>;
}

export function AskUserQuestionWidget({
  toolCall,
  isSubmitting = false,
  onSubmit,
}: AskUserQuestionWidgetProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [selections, setSelections] = useState<Record<number, string[]>>({});
  const [customInputs, setCustomInputs] = useState<Record<number, string>>({});

  const questions = useMemo<Question[]>(() => {
    const raw = toolCall.arguments?.questions;
    return Array.isArray(raw) ? (raw as Question[]) : [];
  }, [toolCall.arguments]);

  const hasSubmitted = toolCall.status === "success";
  const canSubmit =
    questions.length > 0 &&
    questions.every((question, index) => {
      const hasSelection = (selections[index]?.length ?? 0) > 0;
      const hasCustom = Boolean(customInputs[index]?.trim());
      return hasSelection || hasCustom;
    });

  function toggleOption(
    questionIndex: number,
    label: string,
    multiSelect: boolean,
  ) {
    if (hasSubmitted) return;

    setSelections((prev) => {
      const current = prev[questionIndex] ?? [];
      const next = multiSelect
        ? current.includes(label)
          ? current.filter((item) => item !== label)
          : [...current, label]
        : [label];
      return { ...prev, [questionIndex]: next };
    });
    setCustomInputs((prev) => ({
      ...prev,
      [questionIndex]: prev[questionIndex] ?? "",
    }));
  }

  async function handleSubmit() {
    if (isSubmitting || hasSubmitted || !canSubmit) return;

    const answers: Record<string, string | string[]> = {};
    for (const [index, question] of questions.entries()) {
      const customValue = customInputs[index]?.trim();
      const selectedValues = selections[index] ?? [];
      if (customValue) {
        answers[question.header] = customValue;
      } else if (question.multiSelect) {
        answers[question.header] = selectedValues;
      } else if (selectedValues[0]) {
        answers[question.header] = selectedValues[0];
      }
    }

    await onSubmit(JSON.stringify({ answers }));
  }

  return (
    <View
      style={[
        styles.card,
        { borderColor: `${t.warning}55`, backgroundColor: `${t.warning}10` },
      ]}
    >
      <View
        style={[
          styles.header,
          {
            borderBottomColor: `${t.warning}33`,
            backgroundColor: `${t.warning}18`,
          },
        ]}
      >
        <HelpCircle size={15} color={t.warning} strokeWidth={1.8} />
        <Text style={[styles.title, { color: t.warning }]}>Question</Text>
        {hasSubmitted && (
          <View style={styles.badgeRow}>
            <Check size={12} color={t.success} strokeWidth={2.2} />
            <Text style={[styles.badgeText, { color: t.success }]}>
              Answered
            </Text>
          </View>
        )}
      </View>

      <View style={styles.body}>
        {questions.map((question, questionIndex) => {
          const selectedValues = selections[questionIndex] ?? [];
          return (
            <View
              key={`${question.header}-${questionIndex}`}
              style={styles.questionBlock}
            >
              <View style={styles.questionHeader}>
                <Text
                  style={[
                    styles.questionBadge,
                    { color: t.warning, backgroundColor: `${t.warning}22` },
                  ]}
                >
                  {question.header}
                </Text>
              </View>
              <Text style={[styles.questionText, { color: t.foreground }]}>
                {question.question}
              </Text>

              <View style={styles.optionList}>
                {question.options.map((option) => {
                  const selected = selectedValues.includes(option.label);
                  return (
                    <Pressable
                      key={option.label}
                      onPress={() =>
                        toggleOption(
                          questionIndex,
                          option.label,
                          question.multiSelect ?? false,
                        )
                      }
                      disabled={hasSubmitted}
                      style={[
                        styles.option,
                        {
                          borderColor: selected ? t.warning : `${t.border}88`,
                          backgroundColor: selected
                            ? `${t.warning}18`
                            : `${t.card}66`,
                          opacity: hasSubmitted ? 0.7 : 1,
                        },
                      ]}
                    >
                      <View
                        style={[
                          styles.optionIndicator,
                          {
                            borderColor: selected
                              ? t.warning
                              : `${t.mutedForeground}66`,
                            backgroundColor: selected
                              ? t.warning
                              : "transparent",
                            borderRadius: question.multiSelect ? 5 : 10,
                          },
                        ]}
                      >
                        {selected ? (
                          <Check size={10} color="#fff" strokeWidth={3} />
                        ) : null}
                      </View>
                      <View style={styles.optionTextWrap}>
                        <Text
                          style={[styles.optionLabel, { color: t.foreground }]}
                        >
                          {option.label}
                        </Text>
                        {option.description ? (
                          <Text
                            style={[
                              styles.optionDescription,
                              { color: t.mutedForeground },
                            ]}
                          >
                            {option.description}
                          </Text>
                        ) : null}
                      </View>
                    </Pressable>
                  );
                })}

                {!hasSubmitted && (
                  <TextInput
                    value={customInputs[questionIndex] ?? ""}
                    onChangeText={(value) => {
                      setCustomInputs((prev) => ({
                        ...prev,
                        [questionIndex]: value,
                      }));
                      if (value.trim()) {
                        setSelections((prev) => ({
                          ...prev,
                          [questionIndex]: [],
                        }));
                      }
                    }}
                    placeholder="Other (type your answer)..."
                    placeholderTextColor={t.mutedForeground}
                    style={[
                      styles.input,
                      {
                        borderColor: `${t.border}88`,
                        backgroundColor: `${t.card}66`,
                        color: t.foreground,
                      },
                    ]}
                    editable={!hasSubmitted}
                  />
                )}
              </View>
            </View>
          );
        })}

        {!hasSubmitted && (
          <Pressable
            onPress={() => void handleSubmit()}
            disabled={!canSubmit || isSubmitting}
            style={[
              styles.submitButton,
              {
                backgroundColor: t.warning,
                opacity: !canSubmit || isSubmitting ? 0.6 : 1,
              },
            ]}
          >
            <Send size={16} color="#fff" strokeWidth={2} />
            <Text style={styles.submitLabel}>
              {isSubmitting ? "Submitting..." : "Submit Answer"}
            </Text>
          </Pressable>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderRadius: 14,
    borderWidth: 1,
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  title: {
    flex: 1,
    fontSize: 13,
    fontWeight: "600",
  },
  badgeRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  badgeText: {
    fontSize: 11,
    fontWeight: "600",
  },
  body: {
    padding: 14,
    gap: 16,
  },
  questionBlock: {
    gap: 10,
  },
  questionHeader: {
    flexDirection: "row",
    alignItems: "center",
  },
  questionBadge: {
    fontSize: 11,
    fontWeight: "700",
    paddingHorizontal: 8,
    paddingVertical: 4,
    borderRadius: 8,
    overflow: "hidden",
  },
  questionText: {
    fontSize: 14,
    lineHeight: 20,
  },
  optionList: {
    gap: 8,
  },
  option: {
    borderWidth: 1,
    borderRadius: 12,
    padding: 12,
    flexDirection: "row",
    gap: 10,
  },
  optionIndicator: {
    width: 18,
    height: 18,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
    marginTop: 1,
  },
  optionTextWrap: {
    flex: 1,
    gap: 3,
  },
  optionLabel: {
    fontSize: 14,
    fontWeight: "500",
  },
  optionDescription: {
    fontSize: 12,
    lineHeight: 18,
  },
  input: {
    minHeight: 42,
    borderWidth: 1,
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 10,
    fontSize: 14,
  },
  submitButton: {
    minHeight: 44,
    borderRadius: 10,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
  submitLabel: {
    color: "#fff",
    fontSize: 14,
    fontWeight: "600",
  },
});
