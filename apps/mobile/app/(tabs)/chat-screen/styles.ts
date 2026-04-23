import { StyleSheet } from "react-native";

export const styles = StyleSheet.create({
  bootScreen: { flex: 1 },
  bootInner: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 28,
  },
  bootSpinner: {
    marginTop: 20,
  },
  bootActions: {
    marginTop: 24,
    alignItems: "center",
    gap: 12,
    width: "100%",
    maxWidth: 320,
  },
  bootMessage: {
    marginTop: 14,
    fontSize: 15,
    lineHeight: 21,
    textAlign: "center",
  },
  bootButton: {
    marginTop: 4,
    borderRadius: 16,
    paddingVertical: 14,
    paddingHorizontal: 18,
    width: "100%",
    alignItems: "center",
  },
  bootButtonText: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "600",
  },
  bootButtonSecondary: {
    borderRadius: 16,
    borderWidth: StyleSheet.hairlineWidth,
    paddingVertical: 14,
    paddingHorizontal: 18,
    width: "100%",
    alignItems: "center",
  },
  bootButtonSecondaryText: {
    fontSize: 16,
    fontWeight: "600",
  },
  container: { flex: 1 },
  flex: { flex: 1 },
  topBar: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 10,
    gap: 12,
  },
  menuBtn: {
    padding: 4,
  },
  titleBtn: {
    flex: 1,
  },
  title: {
    fontSize: 17,
    fontWeight: "600",
    textAlign: "center",
  },
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  list: {
    paddingHorizontal: 16,
    paddingTop: 8,
  },
  listDesktop: {
    maxWidth: 800,
    alignSelf: "center",
    width: "100%",
  },
  fadeTop: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 64,
  },
  fadeBottom: {
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    height: 120,
  },
  empty: {
    flex: 1,
    justifyContent: "flex-start",
    alignItems: "center",
    paddingTop: "35%",
    gap: 16,
  },
  emptyTitle: {
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  emptyHint: {
    fontSize: 17,
  },
  errorBanner: {
    marginHorizontal: 16,
    marginBottom: 10,
    borderWidth: 1,
    borderRadius: 14,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  errorBannerText: {
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500",
  },
  stubTitle: {
    fontSize: 24,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
  stubText: {
    fontSize: 15,
    marginTop: 8,
  },
  modalBackdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: "rgba(0,0,0,0.6)",
    justifyContent: "flex-end",
    zIndex: 200,
  },
  modelPicker: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: "60%",
    paddingTop: 20,
    paddingBottom: 40,
    backgroundColor: "#1a1f2e",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: "rgba(255,255,255,0.1)",
  },
  modelPickerTitle: {
    fontSize: 18,
    fontWeight: "700",
    textAlign: "center",
    marginBottom: 16,
  },
  modelList: {
    paddingHorizontal: 16,
  },
  modelItem: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderRadius: 12,
    borderWidth: 1,
    marginBottom: 8,
  },
  modelName: {
    fontSize: 16,
    fontWeight: "500",
  },
  modelProvider: {
    fontSize: 13,
    marginTop: 2,
  },
});
