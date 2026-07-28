import {
  useState,
  useRef,
  useEffect,
  useLayoutEffect,
  useMemo,
  useCallback,
  memo,
} from 'react';
import {
  View,
  TextInput,
  Pressable,
  StyleSheet,
  Platform,
  Text,
  Keyboard,
  LayoutAnimation,
  Image,
  Alert,
  useWindowDimensions,
} from 'react-native';
import { BlurView } from '../../platform/blur';
import { Folder, GitBranch, Maximize2, X } from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import * as ImagePicker from '../../platform/image-picker';
import * as DocumentPicker from '../../platform/document-picker';
import * as SecureStore from '../../platform/secure-store';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { manipulateAsync, SaveFormat } from 'expo-image-manipulator';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
  withTiming,
} from 'react-native-reanimated';
import { useThemeContext } from '../../hooks/useTheme';
import { useBreakpoint } from '../../hooks/useBreakpoint';
import { AccordionControls } from './AccordionControls';
import { ChatBarActionButton } from './ChatBarActionButton';
import { ChatBarExpandedEditor } from './ChatBarExpandedEditor';
import { ChatBarModelPopover } from './ChatBarModelPopover';
import { ChatBarRunningLine, RUN_LINE_CORNER_CLIMB } from './ChatBarRunningLine';
import { Waveform } from './Waveform';
import { CrabIcon } from '../ui/CrabIcon';
import { ImagePreviewModal, imagePreviewUri } from './ImagePreviewModal';
import { formatWorkspaceContextMetadata } from './composerMetadata';
import Svg, {
  Circle,
  Path,
  Polygon,
} from 'react-native-svg';
import type { ThinkingLevel, ModelInfo, SessionType } from '@krusty/api';
import type { PermissionMode } from '@krusty/state';

import { ExpoSpeechRecognitionModule, useSpeechRecognitionEvent } from '../../platform/speech';

export interface Attachment {
  uri: string;
  type: 'image' | 'file';
  name: string;
  mimeType?: string;
  base64?: string;
}

interface ChatBarProps {
  /** Keeps unsent text and attachments isolated when mode surfaces remount. */
  draftKey?: string;
  onSend: (content: string, attachments?: Attachment[]) => void;
  onStop: () => void;
  onHeightChange?: (height: number) => void;
  isStreaming: boolean;
  disabled: boolean;
  thinkingLevel: ThinkingLevel;
  onThinkingChange: (level: ThinkingLevel) => void;
  permissionMode: PermissionMode;
  onPermissionModeToggle: () => void;
  fastModeEnabled?: boolean;
  fastModeSupported?: boolean;
  onFastModeToggle?: () => void;
  mode: 'build' | 'plan';
  onModeToggle: () => void;
  onModelSelect: (modelId: string) => void;
  model: string | null;
  models: ModelInfo[];
  sessionType?: SessionType;
  workspaceDirectory?: string | null;
  targetBranch?: string | null;
  tokenCount?: number;
  onOverlayOpenChange?: (open: boolean) => void;
  minimalControls?: boolean;
  /**
   * Desktop: shared chat content band width. Composer + FABs size against this
   * (or measured root width) instead of the full window, so toolbox / resize
   * never clip controls off-screen.
   */
  contentMaxWidth?: number;
}

interface CachedDraft {
  text: string;
  attachments: Attachment[];
}

const MAX_DRAFT_CACHE = 12;
const draftCache = new Map<string, CachedDraft>();

function setDraftCache(key: string, value: CachedDraft) {
  // Strip heavy base64 from drafts so composer history cannot sludge RAM.
  const compact: CachedDraft = {
    text: value.text,
    attachments: value.attachments.map((attachment) => ({
      ...attachment,
      base64: undefined,
    })),
  };
  draftCache.delete(key);
  draftCache.set(key, compact);
  while (draftCache.size > MAX_DRAFT_CACHE) {
    const oldest = draftCache.keys().next().value;
    if (!oldest) break;
    draftCache.delete(oldest);
  }
}


const PILL = 56;
/** Same rounded-square corner as accordion FABs (not a full circle). */
const RADIUS = 18;
const GAP = 10;
const ROOT_HORIZONTAL_PADDING = 10;
const COMPOSER_MAX_HEIGHT = 112;
const INPUT_SIDE_PADDING = 10;
const INPUT_GROWTH_CHROME = 8;
const INPUT_LINE_HEIGHT = 22;
const INPUT_COLLAPSED_MAX_HEIGHT = PILL - 18;
const INPUT_EXPANDED_VERTICAL_PADDING = 8;
const CLOSED_COMPOSER_BOTTOM_GAP = 16;
/** Extra lift on desktop so the bar doesn't hug the window chrome. */
const CLOSED_COMPOSER_BOTTOM_GAP_DESKTOP = 28;
const GAUGE_SIZE = 28;
const GAUGE_TOP_GAP = 4;
const META_ROW_HEIGHT = 24;
const MODEL_POPOVER_MAX_HEIGHT = PILL * 5 + GAP * 4;
const COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH = 8;
/**
 * The accordion responder spans the full composer width so its provider dock
 * can extend left of the FAB column. Keep the model list above that responder:
 * otherwise iOS sends vertical pans to the accordion's GestureDetector instead
 * of the FlatList even though the transparent rows use `box-none`.
 */
const MODEL_POPOVER_Z_INDEX = 45;
/** Matches AccordionControls PROVIDER_PILL_STEP (56 + 8 gap). */
const PROVIDER_PILL_STEP = 64;
/** Gap between provider dock / model list and the bot+crab FAB column. */
const DOCK_TO_FAB_GAP = 10;
const PROVIDER_FILTER_ORDER_KEY = 'krusty-provider-filter-order-v1';
const WEB_INPUT_STYLE = Platform.OS === 'web'
  ? ({
      outlineStyle: 'none',
      outlineWidth: 0,
      resize: 'none',
    } as any)
  : null;

function estimateCompactInputHeight(value: string, inputWidth: number): number {
  if (!value) return 0;
  const charactersPerLine = Math.max(
    12,
    Math.floor(inputWidth / COMPACT_INPUT_AVERAGE_CHARACTER_WIDTH),
  );
  const visualLineCount = value.split('\n').reduce(
    (total, line) =>
      total + Math.max(1, Math.ceil(line.length / charactersPerLine)),
    0,
  );
  return Math.min(
    COMPOSER_MAX_HEIGHT - INPUT_GROWTH_CHROME,
    visualLineCount * INPUT_LINE_HEIGHT,
  );
}

interface ProviderFilter {
  id: string;
  label: string;
}

function parseProviderFilterOrder(raw: string | null): string[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const seen = new Set<string>();
    const order: string[] = [];
    for (const value of parsed) {
      if (typeof value !== 'string') continue;
      const id = value.trim();
      if (!id || seen.has(id)) continue;
      seen.add(id);
      order.push(id);
    }
    return order;
  } catch {
    return [];
  }
}

function uniqueProviderOrder(ids: string[]): string[] {
  const seen = new Set<string>();
  return ids.filter((id) => {
    if (!id || seen.has(id)) return false;
    seen.add(id);
    return true;
  });
}

interface PickedImageAsset {
  uri: string;
  fileName?: string | null;
  mimeType?: string | null;
  base64?: string | null;
}

function normalizeSupportedImageMimeType(
  mimeType?: string | null,
  fileName?: string | null,
  uri?: string | null,
): string | null {
  const normalizedMime = mimeType?.trim().toLowerCase();
  switch (normalizedMime) {
    case 'image/jpeg':
    case 'image/jpg':
    case 'image/pjpeg':
      return 'image/jpeg';
    case 'image/png':
      return 'image/png';
    case 'image/gif':
      return 'image/gif';
    case 'image/webp':
      return 'image/webp';
    default:
      break;
  }

  const candidate = fileName || uri || '';
  const extMatch = candidate.match(/\.([a-z0-9]+)(?:[?#].*)?$/i);
  const ext = extMatch?.[1]?.toLowerCase();
  switch (ext) {
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg';
    case 'png':
      return 'image/png';
    case 'gif':
      return 'image/gif';
    case 'webp':
      return 'image/webp';
    default:
      return null;
  }
}

function buildJpegFileName(fileName: string | null | undefined, fallbackBaseName: string): string {
  const source = fileName?.trim() || fallbackBaseName;
  const baseName = source.replace(/\.[^.]+$/, '');
  return `${baseName || fallbackBaseName}.jpg`;
}

function extensionForImageMimeType(mimeType: string): string {
  switch (mimeType) {
    case 'image/png':
      return 'png';
    case 'image/gif':
      return 'gif';
    case 'image/webp':
      return 'webp';
    default:
      return 'jpg';
  }
}

function defaultImageFileName(mimeType: string, fallbackBaseName: string): string {
  return `${fallbackBaseName}.${extensionForImageMimeType(mimeType)}`;
}

function fileToBase64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : '';
      resolve(result.split(',')[1] ?? '');
    };
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
}

function clipboardImageFileName(file: File, index: number): string {
  const sourceName = file.name?.trim();
  if (sourceName) return sourceName;

  const supportedMimeType = normalizeSupportedImageMimeType(file.type, null, null) ?? 'image/png';
  const suffix = index === 0 ? '' : `-${index + 1}`;
  return `pasted-image${suffix}.${extensionForImageMimeType(supportedMimeType)}`;
}

function normalizeProviderKey(provider: string | null | undefined): string {
  const raw = (provider ?? '').trim().toLowerCase();
  const compact = raw.replace(/[^a-z0-9]/g, '');
  if (!compact) return 'unknown';
  if (compact.includes('openai')) return 'openai';
  if (compact.includes('anthropic') || compact.includes('claude')) return 'anthropic';
  if (compact.includes('openrouter')) return 'openrouter';
  if (compact.includes('minimax')) return 'minimax';
  if (compact === 'xai' || compact.includes('grok')) return 'xai';
  if (compact === 'zai' || compact.includes('zai') || compact.includes('zhipu')) return 'zai';
  return compact;
}

function providerInitials(label: string): string {
  const words = label.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return '?';
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return words.slice(0, 2).map(word => word[0]).join('').toUpperCase();
}

function thinkingDisplayName(level: ThinkingLevel): string {
  switch (level) {
    case 'off':
      return 'Off';
    case 'minimal':
      return 'Minimal';
    case 'low':
      return 'Low';
    case 'medium':
      return 'Medium';
    case 'high':
      return 'High';
    case 'xhigh':
      return 'Extra High';
    case 'max':
      return 'Max';
    case 'ultra':
      return 'Ultra';
    default:
      return 'Thinking';
  }
}

function ProviderLogo({
  providerId,
  label,
  color,
  size = 22,
}: {
  providerId: string;
  label: string;
  color: string;
  size?: number;
}) {
  switch (providerId) {
    case 'openai':
      return (
        <Svg width={size} height={size} viewBox="0 0 20 20">
          <Path
            fill={color}
            d="M11.248 18.25q-.825 0-1.568-.314a4.3 4.3 0 0 1-1.32-.874 4 4 0 0 1-1.304.214 4 4 0 0 1-2.046-.544 4.27 4.27 0 0 1-1.518-1.485 4 4 0 0 1-.56-2.095q0-.48.131-1.04A4.4 4.4 0 0 1 2.04 10.71a4.07 4.07 0 0 1 .017-3.4 4.2 4.2 0 0 1 1.056-1.418 3.8 3.8 0 0 1 1.6-.842 3.9 3.9 0 0 1 .76-1.683q.593-.759 1.451-1.188a4.04 4.04 0 0 1 1.832-.429q.825 0 1.567.313.742.314 1.32.875a4 4 0 0 1 1.304-.215q1.106 0 2.046.545a4.14 4.14 0 0 1 1.501 1.485q.578.941.578 2.095 0 .48-.132 1.04.66.61 1.023 1.419.363.792.363 1.666 0 .892-.38 1.717a4.3 4.3 0 0 1-1.072 1.435 3.8 3.8 0 0 1-1.584.825 3.8 3.8 0 0 1-.775 1.683 4.06 4.06 0 0 1-1.436 1.188 4.04 4.04 0 0 1-1.832.429m-4.076-2.062q.825 0 1.435-.347l3.103-1.782a.36.36 0 0 0 .164-.313v-1.42L7.881 14.62a.67.67 0 0 1-.726 0l-3.118-1.798a.5.5 0 0 1-.017.115v.198q0 .841.396 1.551.413.693 1.139 1.089a3.2 3.2 0 0 0 1.617.412m.165-2.69a.4.4 0 0 0 .181.05q.083 0 .165-.05l1.238-.71-3.977-2.31a.7.7 0 0 1-.363-.643v-3.58q-.825.362-1.32 1.122a2.9 2.9 0 0 0-.495 1.65q0 .809.413 1.55.412.743 1.072 1.123zm3.91 3.663q.875 0 1.585-.396a2.96 2.96 0 0 0 1.534-2.64v-3.564a.32.32 0 0 0-.165-.297l-1.254-.726v4.604a.7.7 0 0 1-.363.643l-3.119 1.799a3 3 0 0 0 1.783.577m.627-6.039V8.878L10.01 7.822 8.129 8.878v2.244l1.881 1.056zM7.057 5.859a.7.7 0 0 1 .363-.644l3.119-1.798a3 3 0 0 0-1.782-.578q-.874 0-1.584.396A2.96 2.96 0 0 0 6.05 4.324a3.07 3.07 0 0 0-.396 1.551v3.547q0 .199.165.314l1.237.726zm8.383 7.887q.825-.364 1.303-1.123.495-.758.495-1.65a3.15 3.15 0 0 0-.412-1.55q-.413-.743-1.073-1.123l-3.086-1.782q-.099-.065-.181-.049a.3.3 0 0 0-.165.05l-1.238.692 3.993 2.327a.6.6 0 0 1 .264.264.64.64 0 0 1 .1.363zm-3.317-8.382a.63.63 0 0 1 .726 0l3.135 1.831v-.297q0-.792-.396-1.501a2.86 2.86 0 0 0-1.105-1.155q-.71-.43-1.65-.43-.825 0-1.436.347L8.294 5.941a.36.36 0 0 0-.165.314v1.418z"
          />
        </Svg>
      );
    case 'anthropic':
      return (
        <Svg width={size} height={size} viewBox="0 0 24 24">
          <Path
            fill={color}
            d="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z"
          />
        </Svg>
      );
    case 'xai':
      return (
        <Svg width={size} height={size} viewBox="0 0 466.04 516.93">
          <Polygon fill={color} points="0.12 182.71 234.14 516.92 338.15 516.92 104.13 182.71 0.12 182.71" />
          <Polygon fill={color} points="0 516.92 104.08 516.92 156.08 442.67 104.04 368.34 0 516.92" />
          <Polygon fill={color} points="466.04 0 361.96 0 182.1 256.86 234.15 331.18 466.04 0" />
          <Polygon fill={color} points="380.78 516.92 466.04 516.92 466.04 37.16 380.78 158.92 380.78 516.92" />
        </Svg>
      );
    case 'minimax':
      return (
        <Svg width={size} height={size} viewBox="0 0 24 24">
          <Path
            fill={color}
            d="M11.43 3.92a.86.86 0 1 0-1.718 0v14.236a1.999 1.999 0 0 1-3.997 0V9.022a.86.86 0 1 0-1.718 0v3.87a1.999 1.999 0 0 1-3.997 0V11.49a.57.57 0 0 1 1.139 0v1.404a.86.86 0 0 0 1.719 0V9.022a1.999 1.999 0 0 1 3.997 0v9.134a.86.86 0 0 0 1.719 0V3.92a1.998 1.998 0 1 1 3.996 0v11.788a.57.57 0 1 1-1.139 0zm10.572 3.105a2 2 0 0 0-1.999 1.997v7.63a.86.86 0 0 1-1.718 0V3.923a1.999 1.999 0 0 0-3.997 0v16.16a.86.86 0 0 1-1.719 0V18.08a.57.57 0 1 0-1.138 0v2a1.998 1.998 0 0 0 3.996 0V3.92a.86.86 0 0 1 1.719 0v12.73a1.999 1.999 0 0 0 3.996 0V9.023a.86.86 0 1 1 1.72 0v6.686a.57.57 0 0 0 1.138 0V9.022a2 2 0 0 0-1.998-1.997"
          />
        </Svg>
      );
    case 'openrouter':
      return (
        <Svg width={size} height={size} viewBox="0 0 24 24">
          <Path
            fill={color}
            d="M16.778 1.844v1.919q-.569-.026-1.138-.032-.708-.008-1.415.037c-1.93.126-4.023.728-6.149 2.237-2.911 2.066-2.731 1.95-4.14 2.75-.396.223-1.342.574-2.185.798-.841.225-1.753.333-1.751.333v4.229s.768.108 1.61.333c.842.224 1.789.575 2.185.799 1.41.798 1.228.683 4.14 2.75 2.126 1.509 4.22 2.11 6.148 2.236.88.058 1.716.041 2.555.005v1.918l7.222-4.168-7.222-4.17v2.176c-.86.038-1.611.065-2.278.021-1.364-.09-2.417-.357-3.979-1.465-2.244-1.593-2.866-2.027-3.68-2.508.889-.518 1.449-.906 3.822-2.59 1.56-1.109 2.614-1.377 3.978-1.466.667-.044 1.418-.017 2.278.02v2.176L24 6.014Z"
          />
        </Svg>
      );
    case 'zai':
      return (
        <Svg width={size} height={Math.round(size * 0.82)} viewBox="0 0 56 46">
          <Path fill={color} d="M29.4256 0.436371L24.2163 7.04244H3.52728L8.73286 0.436371H29.4256Z" />
          <Path fill={color} d="M52.2648 38.5712L47.0592 45.1739H26.4422L31.644 38.5712H52.2648Z" />
          <Path fill={color} d="M55.9614 0.436371L20.7049 45.1742H0.0390625L5.24089 38.5715L16.7845 24.1041L30.0903 7.04244L35.2955 0.436371H55.9614Z" />
        </Svg>
      );
    default:
      return (
        <Text style={[styles.providerInitials, { color }]}>{providerInitials(label)}</Text>
      );
  }
}

async function prepareClipboardImageAttachment(file: File, index: number): Promise<Attachment> {
  const fallbackBaseName = index === 0 ? 'pasted-image' : `pasted-image-${index + 1}`;
  const fileName = clipboardImageFileName(file, index);
  return prepareImageAttachment(
    {
      uri: URL.createObjectURL(file),
      fileName,
      mimeType: file.type || normalizeSupportedImageMimeType(null, fileName, null),
      base64: await fileToBase64(file),
    },
    fallbackBaseName,
  );
}

async function prepareImageAttachment(
  asset: PickedImageAsset,
  fallbackBaseName: string,
): Promise<Attachment> {
  const supportedMimeType = normalizeSupportedImageMimeType(
    asset.mimeType,
    asset.fileName,
    asset.uri,
  );

  if (supportedMimeType && asset.base64) {
    return {
      uri: asset.uri,
      type: 'image',
      name: asset.fileName ?? defaultImageFileName(supportedMimeType, fallbackBaseName),
      mimeType: supportedMimeType,
      base64: asset.base64 ?? undefined,
    };
  }

  try {
    const result = await manipulateAsync(
      asset.uri,
      [],
      { compress: 0.82, format: SaveFormat.JPEG, base64: true },
    );

    if (!result.base64) {
      throw new Error('Image conversion completed without base64 output.');
    }

    return {
      uri: result.uri,
      type: 'image',
      name: buildJpegFileName(asset.fileName, fallbackBaseName),
      mimeType: 'image/jpeg',
      base64: result.base64,
    };
  } catch (error) {
    const reason = error instanceof Error ? error.message : 'Unknown conversion failure.';
    throw new Error(
      `Could not prepare '${asset.fileName ?? fallbackBaseName}' for upload. ${reason}`,
    );
  }
}

function ChatBarComponent(props: ChatBarProps) {
  const {
    draftKey = 'chat',
    onSend, onStop, onHeightChange, isStreaming, disabled,
    thinkingLevel, onThinkingChange,
    permissionMode, onPermissionModeToggle,
    fastModeEnabled, fastModeSupported, onFastModeToggle,
    mode, onModeToggle, onModelSelect, model, models,
    sessionType, workspaceDirectory, targetBranch, tokenCount, onOverlayOpenChange,
    contentMaxWidth, minimalControls = false,
  } = props;

  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const insets = useSafeAreaInsets();
  const { width: viewportWidth, height: viewportHeight } = useWindowDimensions();
  const [text, setText] = useState(
    () => draftCache.get(draftKey)?.text ?? '',
  );
  const [attachments, setAttachments] = useState<Attachment[]>(
    () => draftCache.get(draftKey)?.attachments ?? [],
  );
  const [previewAttachment, setPreviewAttachment] = useState<Attachment | null>(null);
  const [hoveredAttachmentIndex, setHoveredAttachmentIndex] = useState<number | null>(null);
  const [accordionOpen, setAccordionOpen] = useState(false);
  const [accordionVisible, setAccordionVisible] = useState(false);
  const [isRecording, setIsRecording] = useState(false);
  const [micVolume, setMicVolume] = useState(0);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [modelRailOpen, setModelRailOpen] = useState(false);
  const [sortedModels, setSortedModels] = useState<ModelInfo[]>([]);
  const [selectedProviderFilter, setSelectedProviderFilter] = useState<string | null>(null);
  const [providerFilterOrder, setProviderFilterOrder] = useState<string[] | null>(null);
  const [attachPickerOpen, setAttachPickerOpen] = useState(false);
  const [expandedEditorOpen, setExpandedEditorOpen] = useState(false);
  const [inputFocused, setInputFocused] = useState(false);
  const [inputContentHeight, setInputContentHeight] = useState(0);
  const modelPopoverScale = useSharedValue(0);
  const modelPopoverOpacity = useSharedValue(0);
  const [keyboardHeight, setKeyboardHeight] = useState(0);
  const transcriptRef = useRef('');
  const textRef = useRef('');
  const inputRef = useRef<TextInput>(null);
  const accordionOpenRef = useRef(false);
  const measuredRootHeightRef = useRef(0);
  const reportedComposerHeightRef = useRef(0);
  /** Actual laid-out band width (after parent maxWidth / split). */
  const [measuredRootWidth, setMeasuredRootWidth] = useState(0);
  const modelCloseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const draftRef = useRef<CachedDraft>({ text, attachments });
  const activeDraftKeyRef = useRef(draftKey);
  const skipDraftSyncRef = useRef(false);

  useLayoutEffect(() => {
    if (activeDraftKeyRef.current === draftKey) {
      return;
    }

    setDraftCache(activeDraftKeyRef.current, draftRef.current);
    const nextDraft = draftCache.get(draftKey) ?? {
      text: '',
      attachments: [],
    };
    activeDraftKeyRef.current = draftKey;
    draftRef.current = nextDraft;
    skipDraftSyncRef.current = true;
    textRef.current = nextDraft.text;
    setText(nextDraft.text);
    setAttachments(nextDraft.attachments);
    setPreviewAttachment(null);
    setInputContentHeight(0);
    setExpandedEditorOpen(false);
  }, [draftKey]);

  useEffect(() => {
    if (skipDraftSyncRef.current) {
      skipDraftSyncRef.current = false;
      return;
    }
    draftRef.current = { text, attachments };
  }, [attachments, text]);

  useEffect(
    () => () => {
      setDraftCache(activeDraftKeyRef.current, draftRef.current);
    },
    [],
  );

  const clearModelCloseTimer = () => {
    if (!modelCloseTimerRef.current) return;
    clearTimeout(modelCloseTimerRef.current);
    modelCloseTimerRef.current = null;
  };

  useEffect(() => { accordionOpenRef.current = accordionOpen; }, [accordionOpen]);

  useEffect(() => {
    let cancelled = false;
    void SecureStore.getItemAsync(PROVIDER_FILTER_ORDER_KEY)
      .then((raw) => {
        if (cancelled) return;
        setProviderFilterOrder(parseProviderFilterOrder(raw));
      })
      .catch(() => {
        if (!cancelled) setProviderFilterOrder([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => () => {
    clearModelCloseTimer();
  }, []);

  const bottomOverlayOpen =
    accordionVisible ||
    modelPickerOpen ||
    attachPickerOpen ||
    expandedEditorOpen;

  useEffect(() => {
    onOverlayOpenChange?.(bottomOverlayOpen);
  }, [bottomOverlayOpen, onOverlayOpenChange]);

  useEffect(() => () => {
    onOverlayOpenChange?.(false);
  }, [onOverlayOpenChange]);

  useEffect(() => {
    if (accordionOpen) {
      setAccordionVisible(true);
      return;
    }

    if (!accordionVisible) return;
    const timer = setTimeout(() => setAccordionVisible(false), 420);
    return () => clearTimeout(timer);
  }, [accordionOpen, accordionVisible]);

  useEffect(() => {
    if (Platform.OS !== 'web' || !inputFocused || disabled || isRecording) return;

    const handlePaste = (event: ClipboardEvent) => {
      const clipboard = event.clipboardData;
      if (!clipboard) return;

      const itemFiles = Array.from(clipboard.items)
        .filter(item => item.kind === 'file' && item.type.startsWith('image/'))
        .map(item => item.getAsFile())
        .filter((file): file is File => Boolean(file));
      const fileList = itemFiles.length > 0
        ? itemFiles
        : Array.from(clipboard.files).filter(file => file.type.startsWith('image/'));

      if (fileList.length === 0) return;

      event.preventDefault();
      event.stopPropagation();

      void (async () => {
        try {
          const pastedAttachments = await Promise.all(
            fileList.map((file, index) => prepareClipboardImageAttachment(file, index)),
          );
          setAttachments(prev => [...prev, ...pastedAttachments]);
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
        } catch (error) {
          Alert.alert(
            'Could not paste image',
            error instanceof Error ? error.message : 'Image preparation failed.',
          );
        }
      })();
    };

    document.addEventListener('paste', handlePaste, true);
    return () => document.removeEventListener('paste', handlePaste, true);
  }, [disabled, inputFocused, isRecording]);

  const t = theme.colors;
  const isDark = theme.scheme === 'dark';
  const isMako = sessionType === 'mako';
  const showComposerChrome = minimalControls !== true;
  // Match FAB glass so the composer isn't a flat grey strip against the shell.
  const borderColor = t.glass.border;
  const bgOverlay = isDark
    ? 'rgba(14, 20, 30, 0.88)'
    : 'rgba(255, 255, 255, 0.88)';
  const blurTint = isDark ? 'systemChromeMaterialDark' as const : 'systemChromeMaterialLight' as const;
  const pillTint = isDark ? 'systemMaterialDark' as const : 'systemMaterialLight' as const;
  const composerBlur = Math.max(theme.colors.glassBlur ?? 20, isDesktop ? 48 : 40);
  const providerFilters = useMemo<ProviderFilter[]>(() => {
    const seen = new Set<string>();
    const filters: ProviderFilter[] = [];
    for (const modelInfo of sortedModels) {
      const id = normalizeProviderKey(modelInfo.provider);
      if (seen.has(id)) continue;
      seen.add(id);
      filters.push({ id, label: modelInfo.provider });
    }
    return filters;
  }, [sortedModels]);

  const visualProviderFilters = useMemo<ProviderFilter[]>(() => {
    const savedOrder = providerFilterOrder ?? [];
    const byId = new Map(providerFilters.map((provider) => [provider.id, provider]));
    const seen = new Set<string>();
    const ordered: ProviderFilter[] = [];

    for (const id of savedOrder) {
      const provider = byId.get(id);
      if (!provider || seen.has(id)) continue;
      seen.add(id);
      ordered.push(provider);
    }

    // Preserve the current dock's visual order on first load, then append any
    // newly discovered providers at the right edge of the user-controlled dock.
    for (const provider of [...providerFilters].reverse()) {
      if (seen.has(provider.id)) continue;
      seen.add(provider.id);
      ordered.push(provider);
    }

    return ordered;
  }, [providerFilterOrder, providerFilters]);

  useEffect(() => {
    if (providerFilterOrder === null || providerFilterOrder.length > 0 || providerFilters.length === 0) return;
    const initialOrder = [...providerFilters].reverse().map((provider) => provider.id);
    setProviderFilterOrder(initialOrder);
    void SecureStore.setItemAsync(PROVIDER_FILTER_ORDER_KEY, JSON.stringify(initialOrder));
  }, [providerFilterOrder, providerFilters]);

  const handleProviderFiltersReorder = useCallback((providerIds: string[]) => {
    const nextOrder = uniqueProviderOrder(providerIds);
    setProviderFilterOrder(nextOrder);
    void SecureStore.setItemAsync(PROVIDER_FILTER_ORDER_KEY, JSON.stringify(nextOrder));
  }, []);

  const providerFilterActions = useMemo(
    () => visualProviderFilters.map(provider => ({
      id: provider.id,
      label: provider.label,
      icon: (
        <ProviderLogo
          providerId={provider.id}
          label={provider.label}
          color="#fff"
          size={24}
        />
      ),
    })),
    [visualProviderFilters],
  );
  const filteredModels = useMemo(
    () => selectedProviderFilter
      ? sortedModels.filter(modelInfo =>
          normalizeProviderKey(modelInfo.provider) === selectedProviderFilter,
        )
      : sortedModels,
    [selectedProviderFilter, sortedModels],
  );
  const currentModelLabel = useMemo(() => {
    if (!model) return 'No model selected';
    return models.find(candidate => candidate.id === model)?.display_name ?? model;
  }, [model, models]);
  const workspaceContext = useMemo(
    () =>
      sessionType === 'code'
        ? formatWorkspaceContextMetadata(workspaceDirectory, targetBranch)
        : null,
    [sessionType, targetBranch, workspaceDirectory],
  );
  const selectedModelInfo = useMemo(
    () => models.find(candidate => candidate.id === model) ?? null,
    [model, models],
  );
  const thinkingLabel = thinkingDisplayName(thinkingLevel);

  // ── Keyboard tracking with LayoutAnimation ──
  useEffect(() => {
    const showSub = Keyboard.addListener(
      Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow',
      (e) => {
        LayoutAnimation.configureNext(
          LayoutAnimation.create(
            e.duration || 250,
            LayoutAnimation.Types.keyboard,
            LayoutAnimation.Properties.opacity,
          ),
        );
        setKeyboardHeight(e.endCoordinates.height);
      },
    );
    const hideSub = Keyboard.addListener(
      Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide',
      (e) => {
        LayoutAnimation.configureNext(
          LayoutAnimation.create(
            e.duration || 250,
            LayoutAnimation.Types.keyboard,
            LayoutAnimation.Properties.opacity,
          ),
        );
        setKeyboardHeight(0);
        if (accordionOpenRef.current) setAccordionOpen(false);
      },
    );
    return () => { showSub.remove(); hideSub.remove(); };
  }, []);

  // ── Speech recognition ──
  useSpeechRecognitionEvent('result', (event: any) => {
    if (event.isFinal) {
      transcriptRef.current = event.results[0]?.transcript ?? '';
    }
  });
  useSpeechRecognitionEvent('end', () => {
    if (transcriptRef.current) {
      setText(prev => {
        const next = (prev ? prev + ' ' : '') + transcriptRef.current;
        textRef.current = next;
        return next;
      });
      transcriptRef.current = '';
    }
    setIsRecording(false);
    setMicVolume(0);
  });
  useSpeechRecognitionEvent('error', () => {
    transcriptRef.current = '';
    setIsRecording(false);
    setMicVolume(0);
  });
  useSpeechRecognitionEvent('volumechange', (event: any) => {
    // expo-speech-recognition volume is -2 to 10, normalize to 0-1
    const raw = event?.value ?? 0;
    setMicVolume(Math.max(0, Math.min(1, (raw + 2) / 12)));
  });

  const toggleRecording = async () => {
    if (!ExpoSpeechRecognitionModule) return;
    if (isRecording) { ExpoSpeechRecognitionModule.stop(); return; }
    const { granted } = await ExpoSpeechRecognitionModule.requestPermissionsAsync();
    if (!granted) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    transcriptRef.current = '';
    setIsRecording(true);
    ExpoSpeechRecognitionModule.start({ lang: 'en-US', interimResults: false, continuous: false });
  };

  // ── Send ──
  const handleSend = () => {
    if (isStreaming) { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium); onStop(); return; }
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    if (isRecording && ExpoSpeechRecognitionModule) { ExpoSpeechRecognitionModule.stop(); setIsRecording(false); }
    onSend(trimmed, attachments.length > 0 ? attachments : undefined);
    textRef.current = '';
    setText('');
    setInputContentHeight(0);
    setAttachments([]);
    setPreviewAttachment(null);
    setHoveredAttachmentIndex(null);
    setExpandedEditorOpen(false);
    Keyboard.dismiss();
    if (accordionOpen) setAccordionOpen(false);
  };

  // ── Attach ──
  const handleAttach = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    attachPickerOpen ? closeAttachPicker() : openAttachPicker();
  };

  const pickPhoto = async () => {
    closeAttachPicker();
    const r = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ['images'], base64: true, quality: 0.8 });
    if (!r.canceled && r.assets[0]) {
      try {
        const attachment = await prepareImageAttachment(r.assets[0], 'image');
        setAttachments(p => [...p, attachment]);
      } catch (error) {
        Alert.alert(
          'Could not attach image',
          error instanceof Error ? error.message : 'Image preparation failed.',
        );
      }
    }
  };
  const pickCamera = async () => {
    closeAttachPicker();
    const perm = await ImagePicker.requestCameraPermissionsAsync(); if (!perm.granted) return;
    const r = await ImagePicker.launchCameraAsync({ base64: true, quality: 0.8 });
    if (!r.canceled && r.assets[0]) {
      try {
        const attachment = await prepareImageAttachment(r.assets[0], 'photo');
        setAttachments(p => [...p, attachment]);
      } catch (error) {
        Alert.alert(
          'Could not attach image',
          error instanceof Error ? error.message : 'Image preparation failed.',
        );
      }
    }
  };
  const pickFile = async () => {
    closeAttachPicker();
    const r = await DocumentPicker.getDocumentAsync({ type: '*/*' });
    if (!r.canceled && r.assets[0]) { const a = r.assets[0]; setAttachments(p => [...p, { uri: a.uri, type: 'file', name: a.name, mimeType: a.mimeType ?? undefined }]); }
  };

  // Unified action button: mic when empty, send when has text, stop when streaming/recording
  const handleActionBtn = () => {
    if (isStreaming) { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium); onStop(); return; }
    if (isRecording) { toggleRecording(); return; }
    if (canSend) { handleSend(); return; }
    toggleRecording(); // empty state = start recording
  };

  const handleTextChange = (value: string) => {
    textRef.current = value;
    setText(value);
    if (!value) {
      setInputContentHeight(0);
    }
  };

  const toggleAccordion = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    if (accordionOpen) {
      setAccordionOpen(false);
      return;
    }
    setAccordionVisible(true);
    setAccordionOpen(true);
  };

  const openModelPicker = () => {
    clearModelCloseTimer();
    if (attachPickerOpen) {
      setAttachPickerOpen(false);
    }
    // Snapshot sorted order at open time - stays static until closed.
    setSortedModels([...models].sort((a, b) => {
      if (a.id === model) return -1;
      if (b.id === model) return 1;
      return 0;
    }));
    setSelectedProviderFilter(null);
    modelPopoverScale.value = 0;
    modelPopoverOpacity.value = 0;
    setModelRailOpen(true);
    setModelPickerOpen(true);
    modelPopoverScale.value = withSpring(1, { damping: 25, stiffness: 200, mass: 1 });
    modelPopoverOpacity.value = withTiming(1, { duration: 140 });
  };
  const closeModelPicker = () => {
    clearModelCloseTimer();
    setModelRailOpen(false);
    modelPopoverScale.value = withSpring(0, { damping: 25, stiffness: 250 });
    modelPopoverOpacity.value = withTiming(0, { duration: 120 });
    modelCloseTimerRef.current = setTimeout(() => {
      setModelPickerOpen(false);
      setSelectedProviderFilter(null);
      modelCloseTimerRef.current = null;
    }, 180);
  };

  const modelPopoverStyle = useAnimatedStyle(() => ({
    opacity: modelPopoverOpacity.value,
    transform: [{ translateX: (1 - modelPopoverScale.value) * (PILL + GAP) }],
  }));

  const handleModelSelectFromPicker = useCallback((modelId: string) => {
    onModelSelect(modelId);
    closeModelPicker();
  }, [onModelSelect]);

  const closeExpandedEditor = useCallback(() => {
    setExpandedEditorOpen(false);
  }, []);

  const openAttachPicker = () => {
    clearModelCloseTimer();
    if (modelPickerOpen) {
      setModelRailOpen(false);
      modelPopoverScale.value = 0;
      modelPopoverOpacity.value = 0;
      setModelPickerOpen(false);
      setSelectedProviderFilter(null);
    }
    setAttachPickerOpen(true);
  };
  const closeAttachPicker = () => {
    setAttachPickerOpen(false);
  };
  // Close popovers when accordion closes
  useEffect(() => {
    if (!accordionOpen) {
      if (modelPickerOpen) closeModelPicker();
      if (attachPickerOpen) closeAttachPicker();
    }
  }, [accordionOpen]);

  const canSend = !disabled && (isStreaming || text.trim().length > 0 || attachments.length > 0);
  const kActive = accordionOpen || accordionVisible;
  const kColor = kActive ? t.thinking : t.mutedForeground;
  const kBorder = kActive ? t.thinking + '40' : borderColor;

  // Bottom offset: keyboard height, or a safe-area gap when keyboard is closed.
  // Desktop gets extra padding so the bar sits above the window edge cleanly.
  const closedGap = isDesktop
    ? CLOSED_COMPOSER_BOTTOM_GAP_DESKTOP
    : CLOSED_COMPOSER_BOTTOM_GAP;
  const closedBottomOffset = isDesktop
    ? Math.max(closedGap, insets.bottom + 12)
    : Platform.OS === 'web'
      ? Math.max(closedGap, insets.bottom)
      : Math.max(10, Math.min(insets.bottom, closedGap));
  // On iOS/Android, endCoordinates.height is distance from window bottom to keyboard top.
  // Use that directly so the absolute composer sits flush above the keyboard.
  // Avoid subtracting safe-area here or the bar can land in the keyboard.
  const bottomOffset = keyboardHeight > 0 ? keyboardHeight : closedBottomOffset;
  const gaugeTokens = tokenCount ?? 0;
  // Prefer the selected model's real context window (e.g. Grok 500k). Fallback
  // only when the catalog has not loaded or the model is unknown.
  const selectedModel =
    models.find((entry) => entry.id === model) ??
    models.find((entry) => entry.key?.model_id === model) ??
    null;
  const contextWindow = Math.max(
    1,
    selectedModel?.context_window && selectedModel.context_window > 0
      ? selectedModel.context_window
      : 200_000,
  );
  const gaugePct = Math.min(100, (gaugeTokens / contextWindow) * 100);
  const gaugeColor =
    gaugeTokens > contextWindow * 0.9
      ? t.error
      : gaugeTokens > contextWindow * 0.6
        ? t.warning
        : t.mutedForeground + '60';
  const gaugeStroke = 3.5;
  const gaugeRadius = (GAUGE_SIZE - gaugeStroke) / 2;
  const gaugeCircumference = 2 * Math.PI * gaugeRadius;
  const gaugeOffset = gaugeCircumference - (gaugePct / 100) * gaugeCircumference;
  const shouldGrowComposer = inputContentHeight > PILL;
  const composerBarHeight = isRecording || !shouldGrowComposer
    ? PILL
    : Math.min(COMPOSER_MAX_HEIGHT, inputContentHeight + INPUT_GROWTH_CHROME);
  const collapsedInputHeight = Math.max(
    INPUT_LINE_HEIGHT,
    Math.min(inputContentHeight || INPUT_LINE_HEIGHT, INPUT_COLLAPSED_MAX_HEIGHT),
  );
  const metaReserveHeight = showComposerChrome
    ? META_ROW_HEIGHT + GAUGE_TOP_GAP
    : 0;
  // Distance from root bottom to the top of the input/crab row.
  const inputRowBottom = bottomOffset + metaReserveHeight;
  // Accordion sits just above the crab FAB.
  const controlsLayerBottom = showComposerChrome
    ? inputRowBottom + PILL + GAP
    : inputRowBottom;
  // Space above the input row where the model list may sit.
  const overlayBottom = inputRowBottom + composerBarHeight + GAP;
  // The activity line is a screen-edge indicator, independent of the composer
  // safe-area inset. Keep it flush with the bottom edge on every platform.
  const runLineBottom = 0;
  // Prefer measured column width over viewport so split/toolbox/resize stay in-bounds.
  const bandWidth =
    measuredRootWidth > 0
      ? measuredRootWidth
      : contentMaxWidth != null && contentMaxWidth > 0
        ? contentMaxWidth
        : viewportWidth;
  const compactInputWidth = Math.max(
    120,
    bandWidth -
      ROOT_HORIZONTAL_PADDING * 2 -
      PILL -
      GAP -
      36 -
      36 -
      INPUT_SIDE_PADDING * 2,
  );
  const controlsLayerWidth = Math.max(
    PILL,
    bandWidth - ROOT_HORIZONTAL_PADDING * 2,
  );
  const modelPopoverTopInset = Math.max(insets.top, 0) + 12;

  // Chat and Mako share a five-control stack. Code adds Build/Plan as the
  // sixth control. Derive picker geometry from the surface profile rather than
  // keeping a six-row offset that overlaps the provider filters on shorter FABs.
  const hasWorkMode = !showComposerChrome ? false : sessionType === 'code';
  const accordionPillCount = hasWorkMode ? 6 : 5;
  const pillsBelowBot = accordionPillCount - 1;
  const surfaceModelPopoverMaxHeight = Math.max(
    PILL * 2,
    MODEL_POPOVER_MAX_HEIGHT - (hasWorkMode ? 0 : PILL + GAP),
  );
  // Bottom edge of the bot/filter row, from the root bottom.
  const botRowBottom =
    controlsLayerBottom + pillsBelowBot * (PILL + GAP);
  // Pin the *top* of the model list snug under the filter row (10px gap),
  // then grow downward toward the input — no floating mid-gap.
  const listTopGap = 10;
  const desktopModelListMaxHeight = Math.max(
    0,
    botRowBottom - listTopGap - overlayBottom,
  );
  const desktopModelListHeight = Math.min(
    surfaceModelPopoverMaxHeight,
    desktopModelListMaxHeight,
  );
  // top = botRowBottom - listTopGap  ⇒  bottom = top - height
  const desktopModelListBottom =
    botRowBottom - listTopGap - desktopModelListHeight;

  const modelPopoverHeight = isDesktop
    ? desktopModelListHeight
    : Math.min(
        surfaceModelPopoverMaxHeight,
        Math.max(0, viewportHeight - overlayBottom - modelPopoverTopInset),
      );
  // Match desktop filter row: n×56px pills with 8px gaps (no trailing gap).
  const providerCount = Math.max(1, visualProviderFilters.length);
  const providerDockWidth =
    providerCount * 56 + Math.max(0, providerCount - 1) * 8;
  const modelPopoverWidth = isDesktop ? providerDockWidth : undefined;
  // Align under the filter strip (same right edge as filters).
  // controlsLayer is already right-aligned at ROOT_HORIZONTAL_PADDING; bot is the
  // rightmost control, filters sit 8px left of bot — do NOT also add crab width.
  const FILTER_TO_BOT_GAP = 8;
  const dockRightInset = isDesktop
    ? ROOT_HORIZONTAL_PADDING + PILL + FILTER_TO_BOT_GAP
    : ROOT_HORIZONTAL_PADDING + PILL + DOCK_TO_FAB_GAP;

  useEffect(() => {
    if (Platform.OS !== 'web') return;
    const nextHeight = estimateCompactInputHeight(text, compactInputWidth);
    setInputContentHeight((current) =>
      current === nextHeight ? current : nextHeight,
    );
  }, [compactInputWidth, text]);

  useEffect(() => {
    const measuredRootHeight = measuredRootHeightRef.current;
    if (!measuredRootHeight || !onHeightChange) return;
    // Reserve the full mounted height, including keyboard lift. Transcript
    // content must clear both the composer chrome and the open keyboard.
    const reservedHeight = Math.max(PILL, Math.ceil(measuredRootHeight));
    if (reportedComposerHeightRef.current === reservedHeight) return;
    reportedComposerHeightRef.current = reservedHeight;
    onHeightChange(reservedHeight);
  }, [keyboardHeight, onHeightChange]);

  return (
    <View
      pointerEvents="box-none"
      style={[
        styles.root,
        {
          paddingBottom: bottomOffset,
          // Parent desktop column already caps width; fill that band only.
          // Avoid left+right+maxWidth fights on web that ignore the soft-cap.
          ...(contentMaxWidth != null
            ? {
                width: '100%',
                maxWidth: contentMaxWidth,
                alignSelf: 'center' as const,
                left: 0,
                right: undefined,
              }
            : null),
        },
      ]}
      onLayout={(event) => {
        const { height, width } = event.nativeEvent.layout;
        measuredRootHeightRef.current = height;
        const nextWidth = Math.round(width);
        setMeasuredRootWidth((prev) => (prev === nextWidth ? prev : nextWidth));
        if (!onHeightChange) return;
        // Include keyboard paddingBottom so chat content is never covered.
        const reservedHeight = Math.max(PILL, Math.ceil(height));
        if (reportedComposerHeightRef.current === reservedHeight) return;
        reportedComposerHeightRef.current = reservedHeight;
        onHeightChange(reservedHeight);
      }}
    >
      {/* Attachment previews */}
      {attachments.length > 0 && (
        <View style={styles.attachRow}>
          {attachments.map((att, i) => {
            const isImage = att.type === 'image';
            const isHovered = hoveredAttachmentIndex === i;
            const previewUri = imagePreviewUri(att);
            return (
            <View
              key={`${att.name}-${i}`}
              style={[
                styles.attachThumb,
                { borderColor: isImage && isHovered ? t.userMessage : t.border },
              ]}
            >
              {isImage && previewUri
                ? (
                  <Pressable
                    onPress={(event) => {
                      event.stopPropagation();
                      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                      setPreviewAttachment(att);
                    }}
                    onHoverIn={() => setHoveredAttachmentIndex(i)}
                    onHoverOut={() => setHoveredAttachmentIndex(current => current === i ? null : current)}
                    style={({ pressed }) => [
                      styles.attachPreviewButton,
                      pressed && styles.attachPreviewButtonPressed,
                    ]}
                  >
                    <Image source={{ uri: previewUri }} style={styles.attachImg} />
                    <View
                      pointerEvents="none"
                      style={[
                        styles.attachHoverOverlay,
                        {
                          borderColor: t.userMessage,
                          backgroundColor: `${t.userMessage}22`,
                          opacity: isHovered ? 1 : 0,
                        },
                      ]}
                    />
                  </Pressable>
                )
                : <Text style={[styles.attachName, { color: t.mutedForeground }]} numberOfLines={1}>{att.name}</Text>
              }
              <Pressable
                onPress={(event) => {
                  event.stopPropagation();
                  Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  setAttachments(p => p.filter((_, idx) => idx !== i));
                  if (previewAttachment === att) setPreviewAttachment(null);
                  setHoveredAttachmentIndex(current => current === i ? null : current);
                }}
                style={styles.attachX}
              >
                <X size={12} color="#fff" strokeWidth={3} />
              </Pressable>
            </View>
          );
          })}
        </View>
      )}

      {/* L-shape: [input bar] + [K column] */}
      <View style={styles.lRow}>
        {/* Input bar */}
        <View
          style={[
            styles.bar,
            {
              borderColor,
              height: composerBarHeight,
              // Always rounded-square like FABs — not a full capsule/circle.
              borderRadius: RADIUS,
            },
          ]}
        >
          <BlurView intensity={composerBlur} tint={blurTint} style={StyleSheet.absoluteFill} />
          <View style={[StyleSheet.absoluteFill, { backgroundColor: bgOverlay }]} />
          <View style={styles.barInner}>
            {!isRecording ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Expand message editor"
                onPress={() => {
                  void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                  setExpandedEditorOpen(true);
                  setAccordionOpen(false);
                  setAttachPickerOpen(false);
                  setModelPickerOpen(false);
                }}
                style={({ pressed }) => [
                  styles.expandEditorButton,
                  pressed && styles.expandEditorButtonPressed,
                ]}
              >
                <Maximize2
                  size={17}
                  color={t.mutedForeground}
                  strokeWidth={1.8}
                />
              </Pressable>
            ) : null}
            {isRecording
              ? <Waveform active volume={micVolume} />
              : <TextInput
                  ref={inputRef}
                  style={[
                    styles.input,
                    shouldGrowComposer
                      ? styles.inputExpanded
                      : [styles.inputCollapsed, { height: collapsedInputHeight }],
                    WEB_INPUT_STYLE,
                    { color: t.foreground },
                  ]}
                  value={text}
                  onChangeText={handleTextChange}
                  onContentSizeChange={(event) => {
                    if (!textRef.current) {
                      setInputContentHeight(0);
                      return;
                    }
                    const nextHeight = Math.ceil(event.nativeEvent.contentSize.height);
                    setInputContentHeight((current) =>
                      current === nextHeight ? current : nextHeight,
                    );
                  }}
                  onFocus={() => { setInputFocused(true); if (accordionOpen) setAccordionOpen(false); }}
                  onBlur={() => setInputFocused(false)}
                  placeholder={isMako ? "Message Mako..." : "Message Krusty..."}
                  placeholderTextColor={t.mutedForeground + '50'}
                  multiline
                  scrollEnabled={composerBarHeight >= COMPOSER_MAX_HEIGHT}
                  maxLength={500000}
                  editable={!disabled}
                  keyboardAppearance={theme.scheme}
                />
            }
            <ChatBarActionButton
              isStreaming={isStreaming}
              isRecording={isRecording}
              canSend={canSend}
              mutedForeground={t.mutedForeground}
              userMessage={t.userMessage}
              error={t.error}
              onPress={handleActionBtn}
              onLongPress={toggleRecording}
            />
          </View>
        </View>

        {showComposerChrome ? (
          <View style={styles.kCol}>
            <Pressable
              onPress={toggleAccordion}
              style={[
                styles.kWrap,
                {
                  borderColor: kBorder,
                  borderRadius: RADIUS,
                  backgroundColor: kActive ? t.thinking + '14' : undefined,
                },
              ]}
            >
              <BlurView intensity={composerBlur} tint={pillTint} style={StyleSheet.absoluteFill} />
              <View style={[StyleSheet.absoluteFill, { backgroundColor: bgOverlay }]} />
              <View style={styles.kInner}>
                <CrabIcon size={26} color={kColor} />
              </View>
            </Pressable>
          </View>
        ) : null}
      </View>

      {/* Accordion FABs + provider filters: positioned on the root, NOT inside the
          56px crab column (WebKit clips overflow from that narrow column). */}
      {showComposerChrome && accordionVisible ? (
        <View
          pointerEvents="box-none"
          style={[
            styles.controlsLayer,
            {
              bottom: controlsLayerBottom,
              width: controlsLayerWidth,
              right: ROOT_HORIZONTAL_PADDING,
              zIndex: modelPickerOpen || modelRailOpen ? 40 : 20,
            },
          ]}
        >
          <AccordionControls
            thinkingLevel={thinkingLevel}
            onThinkingChange={onThinkingChange}
            permissionMode={permissionMode}
            onPermissionModeToggle={onPermissionModeToggle}
            fastModeEnabled={fastModeEnabled}
            fastModeSupported={fastModeSupported}
            onFastModeToggle={onFastModeToggle}
            mode={mode}
            onModeToggle={onModeToggle}
            onAttach={handleAttach}
            attachPickerOpen={attachPickerOpen}
            onPickPhoto={pickPhoto}
            onPickCamera={pickCamera}
            onPickFile={pickFile}
            onModelSelect={() =>
              modelPickerOpen ? closeModelPicker() : openModelPicker()
            }
            modelPickerOpen={modelRailOpen}
            providerFilters={providerFilterActions}
            selectedProviderFilter={selectedProviderFilter}
            onProviderFiltersReorder={handleProviderFiltersReorder}
            onProviderFilterToggle={(providerId) => {
              setSelectedProviderFilter((current) =>
                current === providerId ? null : providerId,
              );
            }}
            model={model}
            modelInfo={selectedModelInfo}
            isOpen={accordionOpen}
            onToggle={toggleAccordion}
            sessionType={sessionType}
          />
        </View>
      ) : null}

      {/* Model popover — under the filter row, same width + right edge */}
      {showComposerChrome && modelPickerOpen ? (
        <ChatBarModelPopover
          isDesktop={isDesktop}
          modelPopoverWidth={modelPopoverWidth}
          desktopModelListBottom={desktopModelListBottom}
          modelPopoverHeight={modelPopoverHeight}
          dockRightInset={dockRightInset}
          overlayBottom={overlayBottom}
          modelPopoverStyle={modelPopoverStyle}
          borderColor={t.glass.border}
          composerBlur={composerBlur}
          pillTint={pillTint}
          isDark={isDark}
          foreground={t.foreground}
          mutedForeground={t.mutedForeground}
          thinking={t.thinking}
          backgroundElevated={t.glass.backgroundElevated}
          backgroundPressed={t.glass.backgroundPressed}
          filteredModels={filteredModels}
          model={model}
          onSelectModel={handleModelSelectFromPicker}
        />
      ) : null}

      {/* Composer status row — sits in safe area zone below input */}
      <View
        pointerEvents="none"
        style={[styles.metaRow, !isDesktop && styles.metaRowMobile]}
      >
        <View style={styles.metaLeft}>
          <View style={styles.gaugeRing}>
            <Svg width={GAUGE_SIZE} height={GAUGE_SIZE}>
              <Circle cx={GAUGE_SIZE / 2} cy={GAUGE_SIZE / 2} r={gaugeRadius} stroke="rgba(255,255,255,0.06)" strokeWidth={gaugeStroke} fill="none" />
              <Circle cx={GAUGE_SIZE / 2} cy={GAUGE_SIZE / 2} r={gaugeRadius} stroke={gaugeColor} strokeWidth={gaugeStroke} fill="none"
                strokeDasharray={`${gaugeCircumference}`} strokeDashoffset={gaugeOffset} strokeLinecap="round"
                rotation={-90} origin={`${GAUGE_SIZE / 2}, ${GAUGE_SIZE / 2}`}
              />
            </Svg>
            <Text style={[styles.gaugeLabel, { color: t.mutedForeground }]}>
              {gaugeTokens >= 1000 ? `${(gaugeTokens / 1000).toFixed(0)}k` : gaugeTokens}
            </Text>
          </View>
          {workspaceContext ? (
            <View style={styles.metaWorkspace}>
              {workspaceContext.hasBranch ? (
                <GitBranch size={12} color={t.mutedForeground} strokeWidth={1.8} />
              ) : (
                <Folder size={12} color={t.mutedForeground} strokeWidth={1.8} />
              )}
              <Text
                style={[styles.metaWorkspaceText, { color: t.mutedForeground }]}
                numberOfLines={1}
              >
                {workspaceContext.label}
              </Text>
            </View>
          ) : null}
        </View>
        <View style={styles.metaRight}>
          <Text style={[styles.metaModel, { color: t.mutedForeground }]} numberOfLines={1}>
            {currentModelLabel}
          </Text>
          <Text style={[styles.metaDivider, { color: t.mutedForeground }]} numberOfLines={1}>
            |
          </Text>
          <Text style={[styles.metaThinking, { color: t.mutedForeground }]} numberOfLines={1}>
            {thinkingLabel}
          </Text>
        </View>
      </View>
      <ChatBarRunningLine
        active={isStreaming}
        width={bandWidth}
        cornerClimb={isDesktop ? 0 : RUN_LINE_CORNER_CLIMB}
        style={[styles.runLineEdge, { bottom: runLineBottom }]}
      />
      <ImagePreviewModal
        visible={Boolean(previewAttachment)}
        uri={imagePreviewUri(previewAttachment)}
        title={previewAttachment?.name}
        onClose={() => setPreviewAttachment(null)}
      />
      <ChatBarExpandedEditor
        visible={expandedEditorOpen}
        text={text}
        onChangeText={handleTextChange}
        onClose={closeExpandedEditor}
        onSend={handleSend}
        canSend={canSend}
        disabled={disabled}
        placeholder={isMako ? "Message Mako..." : "Message Krusty..."}
        mutedForeground={t.mutedForeground}
        foreground={t.foreground}
        userMessage={t.userMessage}
        border={t.border}
        keyboardAppearance={theme.scheme}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    paddingHorizontal: ROOT_HORIZONTAL_PADDING,
    paddingTop: 6,
  },
  attachRow: { flexDirection: 'row', gap: 8, marginBottom: 8, paddingLeft: 4 },
  attachThumb: { width: 52, height: 52, borderRadius: 10, borderWidth: StyleSheet.hairlineWidth, overflow: 'hidden', justifyContent: 'center', alignItems: 'center' },
  attachPreviewButton: { width: '100%', height: '100%' },
  attachPreviewButtonPressed: { opacity: 0.82 },
  attachImg: { width: '100%', height: '100%' },
  attachHoverOverlay: { ...StyleSheet.absoluteFillObject, borderRadius: 10, borderWidth: 2 },
  attachName: { fontSize: 9, paddingHorizontal: 3, textAlign: 'center' },
  attachX: { position: 'absolute', top: 2, right: 2, width: 16, height: 16, borderRadius: 8, backgroundColor: 'rgba(0,0,0,0.6)', justifyContent: 'center', alignItems: 'center', zIndex: 2 },
  runLineEdge: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    borderRadius: 0,
    zIndex: 2,
  },
  lRow: { flexDirection: 'row', alignItems: 'flex-end', gap: GAP, minHeight: PILL },
  bar: {
    flex: 1,
    height: PILL,
    maxHeight: COMPOSER_MAX_HEIGHT,
    borderRadius: RADIUS,
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
    // Subtle depth so the composer reads as a FAB dock, not a grey slab.
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.22,
    shadowRadius: 18,
    elevation: 8,
  },
  barInner: {
    flexDirection: 'row',
    alignItems: 'center',
    height: '100%',
    paddingHorizontal: INPUT_SIDE_PADDING,
    paddingVertical: 0,
    gap: 4,
  },
  expandEditorButton: {
    width: 32,
    height: 36,
    borderRadius: 12,
    alignItems: 'center',
    justifyContent: 'center',
  },
  expandEditorButtonPressed: {
    backgroundColor: 'rgba(255,255,255,0.06)',
  },
  btn: { width: 34, height: 34, borderRadius: 17, justifyContent: 'center', alignItems: 'center' },
  input: {
    flex: 1,
    fontSize: 16,
    lineHeight: INPUT_LINE_HEIGHT,
    maxHeight: COMPOSER_MAX_HEIGHT,
    paddingVertical: 0,
    paddingHorizontal: 6,
  },
  inputCollapsed: {
    minHeight: INPUT_LINE_HEIGHT,
    textAlignVertical: 'center',
  },
  inputExpanded: {
    height: '100%',
    paddingTop: INPUT_EXPANDED_VERTICAL_PADDING,
    paddingBottom: INPUT_EXPANDED_VERTICAL_PADDING,
    textAlignVertical: 'top',
  },
  kCol: {
    width: PILL,
    height: PILL,
    alignItems: 'center',
    justifyContent: 'flex-end',
    overflow: 'visible',
    position: 'relative',
    zIndex: 15,
  },
  controlsLayer: {
    position: 'absolute',
    // bottom/right/width set inline from layout metrics
    alignItems: 'flex-end',
    overflow: 'visible',
  },
  kWrap: {
    width: PILL,
    height: PILL,
    // Rounded square — matches accordion FAB pills (not a circle).
    borderRadius: RADIUS,
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
    shadowColor: '#000',
    shadowOffset: { width: 0, height: 6 },
    shadowOpacity: 0.2,
    shadowRadius: 14,
    elevation: 8,
  },
  kInner: { flex: 1, justifyContent: 'center', alignItems: 'center' },

  providerInitials: {
    fontSize: 11,
    fontWeight: '800',
    letterSpacing: 0,
  },
  // Composer status row
  metaRow: {
    height: META_ROW_HEIGHT + GAUGE_TOP_GAP,
    paddingTop: GAUGE_TOP_GAP,
    paddingHorizontal: 4,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  metaRowMobile: {
    paddingHorizontal: 26,
  },
  metaLeft: {
    flex: 1,
    maxWidth: '54%',
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  metaRight: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    gap: 10,
  },
  metaWorkspace: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 5,
  },
  metaWorkspaceText: {
    flex: 1,
    minWidth: 0,
    fontSize: 11,
    fontWeight: '600',
    letterSpacing: 0,
  },
  metaModel: {
    flexShrink: 1,
    minWidth: 0,
    fontSize: 11,
    fontWeight: '700',
    letterSpacing: 0,
    textAlign: 'right',
  },
  metaDivider: {
    flexShrink: 0,
    fontSize: 11,
    fontWeight: '600',
    letterSpacing: 0,
  },
  metaThinking: {
    flexShrink: 0,
    fontSize: 11,
    fontWeight: '700',
    letterSpacing: 0,
  },
  gaugeRing: { width: GAUGE_SIZE, height: GAUGE_SIZE, alignItems: 'center', justifyContent: 'center' },
  gaugeLabel: { position: 'absolute', fontSize: 7, fontWeight: '600', letterSpacing: 0 },
});

export const ChatBar = memo(ChatBarComponent);
