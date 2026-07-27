import { StyleSheet } from "react-native";

/**
 * Desktop chat band: fills available pane width with light side breathing
 * room, soft-capped only on ultra-wide monitors so messages + composer stay
 * one aligned column (no full-bleed ultrawide void).
 */
export const DESKTOP_CHAT_MIN_WIDTH = 360;
export const DESKTOP_CHAT_MAX_WIDTH = 960;
/** Side inset as a fraction of available pane width (clamped). */
export const DESKTOP_CHAT_SIDE_PAD_MIN = 16;
export const DESKTOP_CHAT_SIDE_PAD_MAX = 48;
/** Fixed toolbox side rail — never flex-share with chat. */
export const TOOLBOX_DOCK_WIDTH = 360;

/** Side pad for a measured pane width. */
export function resolveDesktopChatSidePad(paneWidth: number): number {
  if (paneWidth <= 0) return DESKTOP_CHAT_SIDE_PAD_MIN;
  return Math.min(
    DESKTOP_CHAT_SIDE_PAD_MAX,
    Math.max(DESKTOP_CHAT_SIDE_PAD_MIN, Math.round(paneWidth * 0.03)),
  );
}

/**
 * Resolve column max width for a pane width.
 * Always returns a positive width so the band never full-bleeds while waiting
 * on layout measure (pass window width as fallback).
 */
export function resolveDesktopChatMaxWidth(paneWidth: number): number {
  if (paneWidth <= 0) return DESKTOP_CHAT_MAX_WIDTH;
  const pad = resolveDesktopChatSidePad(paneWidth);
  const usable = Math.max(DESKTOP_CHAT_MIN_WIDTH, paneWidth - pad * 2);
  return Math.min(DESKTOP_CHAT_MAX_WIDTH, usable);
}

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
  /** Desktop: chat column + toolbox column side by side. */
  desktopSplit: {
    flex: 1,
    flexDirection: "row",
    minWidth: 0,
    overflow: "hidden",
  },
  /** Flex child that holds the chat band; must shrink when toolbox docks. */
  desktopSplitChat: {
    flex: 1,
    minWidth: 0,
    overflow: "hidden",
  },
  /**
   * Fluid band for messages + composer only. maxWidth set inline from pane.
   * Title chrome stays full-pane so the toolbox button can sit in the corner.
   */
  desktopChatColumn: {
    flex: 1,
    width: "100%",
    maxWidth: DESKTOP_CHAT_MAX_WIDTH,
    alignSelf: "center",
    minWidth: 0,
    // Anchor absolute ChatBar/FABs inside this band (not the full window).
    position: "relative",
    overflow: "visible",
  },
  /** Outer measure host — full split-pane width; chrome + centered band stack. */
  desktopChatColumnHost: {
    flex: 1,
    width: "100%",
    minWidth: 0,
    flexDirection: "column",
    alignItems: "stretch",
  },
  topBar: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 10,
    gap: 12,
  },
  /** Desktop chrome: full pane width so corner controls stay on the window edge. */
  topBarDesktop: {
    width: "100%",
    paddingLeft: 56, // room for shell sidebar-open control
    paddingRight: 12,
    paddingVertical: 10,
    zIndex: 20,
  },
  menuBtn: {
    padding: 4,
  },
  topBarActions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  /** Flush top-right toolbox control (true corner hit target). */
  toolboxCornerBtn: {
    width: 40,
    height: 40,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  titleBtn: {
    maxWidth: "70%",
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 16,
    paddingVertical: 8,
    flex: 1,
    minWidth: 0,
  },
  title: {
    fontSize: 16,
    fontWeight: "700",
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
    maxWidth: DESKTOP_CHAT_MAX_WIDTH,
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
  renameBackdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.55)",
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 24,
  },
  renameCard: {
    width: "100%",
    maxWidth: 420,
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 18,
    gap: 14,
  },
  renameTitle: {
    fontSize: 18,
    fontWeight: "700",
    letterSpacing: -0.2,
  },
  renameInput: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    paddingHorizontal: 14,
    paddingVertical: 12,
    fontSize: 16,
    fontWeight: "600",
  },
  renameActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: 10,
  },
  renameButton: {
    minWidth: 88,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    paddingVertical: 10,
    alignItems: "center",
  },
  renameButtonPrimary: {
    borderWidth: 0,
  },
  renameButtonText: {
    fontSize: 14,
    fontWeight: "700",
  },
});
