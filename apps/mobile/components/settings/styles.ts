import { StyleSheet } from "react-native";

export const styles = StyleSheet.create({
  content: {
    padding: 20,
    paddingBottom: 48,
    gap: 14,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 4,
  },
  title: {
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  closeBtn: {
    width: 34,
    height: 34,
    alignItems: "center",
    justifyContent: "center",
  },
  sectionHeader: {
    gap: 2,
    marginTop: 4,
  },
  sectionTitle: {
    fontSize: 14,
    fontWeight: "600",
  },
  sectionSubtitle: {
    fontSize: 12,
    lineHeight: 18,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  rowContent: {
    flex: 1,
    gap: 2,
  },
  rowTitle: {
    fontSize: 16,
    fontWeight: "600",
  },
  rowSubtitle: {
    fontSize: 13,
    lineHeight: 18,
  },
  separator: {
    height: StyleSheet.hairlineWidth,
    marginVertical: 12,
  },
  actions: {
    flexDirection: "row",
    gap: 18,
    flexWrap: "wrap",
  },
  actionBtn: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  actionText: {
    fontSize: 15,
    fontWeight: "600",
  },
  connectForm: {
    gap: 14,
  },
  inputWrap: {
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 12,
  },
  inputLabel: {
    fontSize: 11,
    fontWeight: "600",
    marginBottom: 6,
  },
  input: {
    fontSize: 16,
    padding: 0,
  },
  inlineInput: {
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    paddingVertical: 12,
  },
  errorText: {
    fontSize: 13,
    lineHeight: 18,
  },
  connectBtn: {
    borderRadius: 10,
    paddingVertical: 14,
    alignItems: "center",
  },
  connectBtnText: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "700",
  },
  schemeRow: {
    flexDirection: "row",
    gap: 10,
    flexWrap: "wrap",
  },
  schemeBtn: {
    minWidth: 110,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
    paddingVertical: 10,
    paddingHorizontal: 14,
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
  },
  schemeBtnText: {
    fontSize: 14,
    fontWeight: "600",
  },
  banner: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  bannerText: {
    fontSize: 13,
    lineHeight: 18,
  },
  loadingRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  loadingText: {
    fontSize: 14,
  },
  stack: {
    gap: 12,
  },
  subsection: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    padding: 12,
    gap: 10,
  },
  subsectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  compactHeader: {
    marginTop: 2,
  },
  pillRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    flexWrap: "wrap",
  },
  pill: {
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderRadius: 8,
  },
  pillText: {
    fontSize: 11,
    fontWeight: "600",
  },
  actionsWrap: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
  },
  smallActionBtn: {
    minHeight: 36,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  smallActionText: {
    fontSize: 13,
    fontWeight: "600",
  },
  oauthCodeHint: {
    fontSize: 15,
    fontWeight: "700",
  },
  twoCol: {
    flexDirection: "row",
    gap: 10,
  },
  toggle: {
    width: 48,
    borderRadius: 12,
    padding: 3,
  },
  toggleKnob: {
    width: 20,
    height: 20,
    borderRadius: 8,
    backgroundColor: "#fff",
  },
  portRow: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 12,
    gap: 8,
  },
  emptyText: {
    fontSize: 14,
    lineHeight: 20,
  },
});
