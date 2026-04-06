import { useState, useRef, useEffect } from 'react';
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
  FlatList,
} from 'react-native';
import { BlurView } from '../../platform/blur';
import { ArrowUp, Square, X, Mic, FlaskConical } from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import * as ImagePicker from '../../platform/image-picker';
import * as DocumentPicker from '../../platform/document-picker';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
} from 'react-native-reanimated';
import { useThemeContext } from '../../hooks/useTheme';
import { AccordionControls } from './AccordionControls';
import { Waveform } from './Waveform';
import { CrabIcon } from '../ui/CrabIcon';
import Svg, { Circle } from 'react-native-svg';
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
  onSend: (content: string, attachments?: Attachment[]) => void;
  onStop: () => void;
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
  researchEnabled?: boolean;
  onResearchToggle?: () => void;
  tokenCount?: number;
}

const PILL = 56;
const RADIUS = 18;
const GAP = 10;

export function ChatBar(props: ChatBarProps) {
  const {
    onSend, onStop, isStreaming, disabled,
    thinkingLevel, onThinkingChange,
    permissionMode, onPermissionModeToggle,
    fastModeEnabled, fastModeSupported, onFastModeToggle,
    mode, onModeToggle, onModelSelect, model, models,
    sessionType, researchEnabled, onResearchToggle, tokenCount,
  } = props;

  const { theme } = useThemeContext();
  const insets = useSafeAreaInsets();
  const [text, setText] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [accordionOpen, setAccordionOpen] = useState(false);
  const [isRecording, setIsRecording] = useState(false);
  const [micVolume, setMicVolume] = useState(0);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [sortedModels, setSortedModels] = useState<ModelInfo[]>([]);
  const [attachPickerOpen, setAttachPickerOpen] = useState(false);
  const modelPopoverScale = useSharedValue(0);
  const modelPopoverOpacity = useSharedValue(0);
  const attachPopoverScale = useSharedValue(0);
  const attachPopoverOpacity = useSharedValue(0);
  const [keyboardHeight, setKeyboardHeight] = useState(0);
  const transcriptRef = useRef('');
  const inputRef = useRef<TextInput>(null);
  const accordionOpenRef = useRef(false);
  useEffect(() => { accordionOpenRef.current = accordionOpen; }, [accordionOpen]);

  const t = theme.colors;
  const isDark = theme.scheme === 'dark';
  const borderColor = 'rgba(255,255,255,0.08)';
  const bgOverlay = isDark ? 'rgba(11,17,25,0.6)' : 'rgba(255,255,255,0.6)';
  const blurTint = isDark ? 'systemChromeMaterialDark' as const : 'systemChromeMaterialLight' as const;
  const pillTint = isDark ? 'systemMaterialDark' as const : 'systemMaterialLight' as const;

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
      setText(prev => (prev ? prev + ' ' : '') + transcriptRef.current);
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
    setText('');
    setAttachments([]);
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
    if (!r.canceled && r.assets[0]) { const a = r.assets[0]; setAttachments(p => [...p, { uri: a.uri, type: 'image', name: a.fileName ?? 'image.jpg', mimeType: a.mimeType ?? 'image/jpeg', base64: a.base64 ?? undefined }]); }
  };
  const pickCamera = async () => {
    closeAttachPicker();
    const perm = await ImagePicker.requestCameraPermissionsAsync(); if (!perm.granted) return;
    const r = await ImagePicker.launchCameraAsync({ base64: true, quality: 0.8 });
    if (!r.canceled && r.assets[0]) { const a = r.assets[0]; setAttachments(p => [...p, { uri: a.uri, type: 'image', name: a.fileName ?? 'photo.jpg', mimeType: a.mimeType ?? 'image/jpeg', base64: a.base64 ?? undefined }]); }
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

  const toggleAccordion = () => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); setAccordionOpen(!accordionOpen); };

  const openModelPicker = () => {
    // Close attach picker if open
    if (attachPickerOpen) closeAttachPicker();
    // Snapshot sorted order at open time — stays static until closed
    setSortedModels([...models].sort((a, b) => {
      if (a.id === model) return -1;
      if (b.id === model) return 1;
      return 0;
    }));
    setModelPickerOpen(true);
    modelPopoverScale.value = withSpring(1, { damping: 25, stiffness: 200, mass: 1 });
    modelPopoverOpacity.value = withSpring(1, { damping: 25, stiffness: 200 });
  };
  const closeModelPicker = () => {
    modelPopoverScale.value = withSpring(0, { damping: 25, stiffness: 250 });
    modelPopoverOpacity.value = withSpring(0, { damping: 25, stiffness: 250 });
    setTimeout(() => setModelPickerOpen(false), 250);
  };

  const modelPopoverStyle = useAnimatedStyle(() => ({
    opacity: modelPopoverOpacity.value,
    transform: [{ translateX: (1 - modelPopoverScale.value) * (PILL + GAP) }],
  }));

  const openAttachPicker = () => {
    // Close model picker if open
    if (modelPickerOpen) closeModelPicker();
    setAttachPickerOpen(true);
    attachPopoverScale.value = withSpring(1, { damping: 25, stiffness: 200, mass: 1 });
    attachPopoverOpacity.value = withSpring(1, { damping: 25, stiffness: 200 });
  };
  const closeAttachPicker = () => {
    attachPopoverScale.value = withSpring(0, { damping: 25, stiffness: 250 });
    attachPopoverOpacity.value = withSpring(0, { damping: 25, stiffness: 250 });
    setTimeout(() => setAttachPickerOpen(false), 250);
  };
  const attachPopoverStyle = useAnimatedStyle(() => ({
    opacity: attachPopoverOpacity.value,
    transform: [{ translateX: (1 - attachPopoverScale.value) * (PILL + GAP) }],
  }));
  // Close popovers when accordion closes
  useEffect(() => {
    if (!accordionOpen) {
      if (modelPickerOpen) closeModelPicker();
      if (attachPickerOpen) closeAttachPicker();
    }
  }, [accordionOpen]);

  const canSend = !disabled && (isStreaming || text.trim().length > 0 || attachments.length > 0);
  const kColor = accordionOpen ? t.thinking : t.mutedForeground;
  const kBorder = accordionOpen ? t.thinking + '40' : borderColor;

  // Bottom offset: keyboard height, or safe area when keyboard closed
  const bottomOffset = keyboardHeight > 0 ? keyboardHeight : insets.bottom;
  const gaugeTokens = tokenCount ?? 0;
  const gaugePct = Math.min(100, (gaugeTokens / 200000) * 100);
  const gaugeColor = gaugeTokens > 180000 ? t.error : gaugeTokens > 120000 ? t.warning : t.mutedForeground + '60';

  return (
    <View style={[styles.root, { paddingBottom: bottomOffset }]}>
      {/* Attachment previews */}
      {attachments.length > 0 && (
        <View style={styles.attachRow}>
          {attachments.map((att, i) => (
            <View key={i} style={[styles.attachThumb, { borderColor: t.border }]}>
              {att.type === 'image'
                ? <Image source={{ uri: att.uri }} style={styles.attachImg} />
                : <Text style={[styles.attachName, { color: t.mutedForeground }]} numberOfLines={1}>{att.name}</Text>
              }
              <Pressable onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); setAttachments(p => p.filter((_, idx) => idx !== i)); }} style={styles.attachX}>
                <X size={12} color="#fff" strokeWidth={3} />
              </Pressable>
            </View>
          ))}
        </View>
      )}

      {/* L-shape: [input bar] + [K column] */}
      <View style={styles.lRow}>
        {/* Input bar */}
        <View style={[styles.bar, { borderColor }]}>
          <BlurView intensity={40} tint={blurTint} style={StyleSheet.absoluteFill} />
          <View style={[StyleSheet.absoluteFill, { backgroundColor: bgOverlay }]} />
          <View style={styles.barInner}>
            {isRecording
              ? <Waveform active volume={micVolume} />
              : <TextInput
                  ref={inputRef}
                  style={[styles.input, { color: t.foreground }]}
                  value={text}
                  onChangeText={setText}
                  onFocus={() => { if (accordionOpen) setAccordionOpen(false); }}
                  placeholder="Message Krusty..."
                  placeholderTextColor={t.mutedForeground + '50'}
                  multiline
                  maxLength={500000}
                  editable={!disabled}
                  keyboardAppearance={theme.scheme}
                />
            }
            <Pressable
              onPress={handleActionBtn}
              onLongPress={canSend ? toggleRecording : undefined}
              delayLongPress={300}
              style={({ pressed }) => [styles.actionBtn, {
                backgroundColor: isRecording
                  ? t.error
                  : canSend
                    ? pressed ? t.userMessage + 'cc' : isStreaming ? t.error : t.userMessage
                    : 'transparent',
              }]}
            >
              {isStreaming
                ? <Square size={14} color="#fff" fill="#fff" strokeWidth={0} />
                : isRecording
                  ? <Square size={14} color="#fff" fill="#fff" strokeWidth={0} />
                  : canSend
                    ? <ArrowUp size={18} color="#fff" strokeWidth={2.5} />
                    : <Mic size={18} color={t.mutedForeground} strokeWidth={1.8} />
              }
            </Pressable>
          </View>
        </View>

        {/* K column */}
        <View style={styles.kCol}>
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
            onModelSelect={() => modelPickerOpen ? closeModelPicker() : openModelPicker()}
            model={model}
            isOpen={accordionOpen}
            onToggle={toggleAccordion}
            sessionType={sessionType}
            researchEnabled={researchEnabled}
            onResearchToggle={onResearchToggle}
          />
          <Pressable onPress={toggleAccordion} style={styles.kWrap}>
            <BlurView intensity={20} tint={pillTint} style={StyleSheet.absoluteFill} />
            <View style={[StyleSheet.absoluteFill, { backgroundColor: bgOverlay }]} />
            <View style={[styles.kInner, { borderColor: kBorder }]}>
              <CrabIcon size={26} color={kColor} />
            </View>
          </Pressable>
        </View>
      </View>

      {/* Model popover — slides out from behind accordion */}
      {modelPickerOpen && (
          <View style={[styles.modelClip, { bottom: bottomOffset + PILL + GAP }]}>
            <Animated.View style={[styles.modelPopover, modelPopoverStyle]}>
            <BlurView intensity={40} tint={pillTint} style={StyleSheet.absoluteFill} />
            <View style={[StyleSheet.absoluteFill, { backgroundColor: isDark ? 'rgba(15,20,30,0.92)' : 'rgba(255,255,255,0.92)', borderRadius: RADIUS }]} />
            <FlatList
              data={sortedModels}
              keyExtractor={(m: ModelInfo) => m.id}
              style={styles.modelList}
              showsVerticalScrollIndicator={false}
              renderItem={({ item }: { item: ModelInfo }) => (
                <Pressable
                  onPress={() => {
                    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                    onModelSelect(item.id);
                  }}
                  style={styles.modelItem}
                >
                  <View style={styles.modelRow}>
                    <View style={styles.modelInfo}>
                      <Text style={[styles.modelName, { color: t.foreground }]} numberOfLines={1}>
                        {item.display_name}
                      </Text>
                      <Text style={[styles.modelMeta, { color: t.mutedForeground }]}>{item.provider}</Text>
                    </View>
                    {item.id === model && (
                      <Text style={[styles.modelCheck, { color: t.thinking }]}>✓</Text>
                    )}
                  </View>
                </Pressable>
              )}
            />
            </Animated.View>
          </View>
      )}

      {/* Attach popover — same area, slides from behind accordion */}
      {attachPickerOpen && (
          <View style={[styles.modelClip, { bottom: bottomOffset + PILL + GAP }]}>
            <Animated.View style={[styles.modelPopover, attachPopoverStyle]}>
              <BlurView intensity={40} tint={pillTint} style={StyleSheet.absoluteFill} />
              <View style={[StyleSheet.absoluteFill, { backgroundColor: isDark ? 'rgba(15,20,30,0.92)' : 'rgba(255,255,255,0.92)', borderRadius: RADIUS }]} />
              <View style={styles.attachOptions}>
                <Pressable onPress={pickPhoto} style={styles.attachOption}>
                  <Text style={[styles.attachOptionText, { color: t.foreground }]}>Photos</Text>
                </Pressable>
                <Pressable onPress={pickCamera} style={styles.attachOption}>
                  <Text style={[styles.attachOptionText, { color: t.foreground }]}>Camera</Text>
                </Pressable>
                <Pressable onPress={pickFile} style={styles.attachOption}>
                  <Text style={[styles.attachOptionText, { color: t.foreground }]}>Files</Text>
                </Pressable>
              </View>
            </Animated.View>
          </View>
      )}

      {/* Context ring gauge — sits in safe area zone below input */}
      {gaugeTokens > 0 && (() => {
        const size = 28;
        const stroke = 3.5;
        const r = (size - stroke) / 2;
        const circ = 2 * Math.PI * r;
        const offset = circ - (gaugePct / 100) * circ;
        return (
          <View style={styles.gaugeRow}>
            <View style={styles.gaugeRing}>
              <Svg width={size} height={size}>
                <Circle cx={size / 2} cy={size / 2} r={r} stroke="rgba(255,255,255,0.06)" strokeWidth={stroke} fill="none" />
                <Circle cx={size / 2} cy={size / 2} r={r} stroke={gaugeColor} strokeWidth={stroke} fill="none"
                  strokeDasharray={`${circ}`} strokeDashoffset={offset} strokeLinecap="round"
                  rotation={-90} origin={`${size / 2}, ${size / 2}`}
                />
              </Svg>
              <Text style={[styles.gaugeLabel, { color: t.mutedForeground }]}>
                {gaugeTokens >= 1000 ? `${(gaugeTokens / 1000).toFixed(0)}k` : gaugeTokens}
              </Text>
            </View>
          </View>
        );
      })()}
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    paddingHorizontal: 10,
    paddingTop: 6,
  },
  attachRow: { flexDirection: 'row', gap: 8, marginBottom: 8, paddingLeft: 4 },
  attachThumb: { width: 52, height: 52, borderRadius: 10, borderWidth: StyleSheet.hairlineWidth, overflow: 'hidden', justifyContent: 'center', alignItems: 'center' },
  attachImg: { width: '100%', height: '100%' },
  attachName: { fontSize: 9, paddingHorizontal: 3, textAlign: 'center' },
  attachX: { position: 'absolute', top: 2, right: 2, width: 16, height: 16, borderRadius: 8, backgroundColor: 'rgba(0,0,0,0.6)', justifyContent: 'center', alignItems: 'center' },
  lRow: { flexDirection: 'row', alignItems: 'flex-end', gap: GAP },
  bar: { flex: 1, borderRadius: RADIUS, overflow: 'hidden', borderWidth: StyleSheet.hairlineWidth, minHeight: PILL },
  barInner: { flexDirection: 'row', alignItems: 'center', paddingHorizontal: 8, paddingVertical: 8, minHeight: PILL, gap: 4 },
  btn: { width: 34, height: 34, borderRadius: 17, justifyContent: 'center', alignItems: 'center' },
  actionBtn: { width: 36, height: 36, borderRadius: 18, justifyContent: 'center', alignItems: 'center' },
  input: { flex: 1, fontSize: 16, lineHeight: 22, maxHeight: 120, paddingVertical: Platform.OS === 'ios' ? 6 : 4, paddingHorizontal: 6 },
  kCol: { width: PILL, alignItems: 'center', justifyContent: 'flex-end', gap: GAP },
  kWrap: { width: PILL, height: PILL, borderRadius: RADIUS, overflow: 'hidden', borderWidth: StyleSheet.hairlineWidth, borderColor: 'rgba(255,255,255,0.08)' },
  kInner: { flex: 1, justifyContent: 'center', alignItems: 'center' },

  // Clip container — hides the popover as it slides from behind accordion
  modelClip: {
    position: 'absolute',
    left: 10,
    right: PILL + GAP + 10,
    height: 4 * PILL + 3 * GAP,
    overflow: 'hidden',
  },
  // Model popover — slides out from behind accordion
  modelPopover: {
    width: '100%',
    height: '100%',
    borderRadius: RADIUS,
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: 'rgba(255,255,255,0.1)',
  },
  modelList: { padding: 8 },
  modelItem: {
    paddingHorizontal: 14,
    paddingVertical: 12,
    borderRadius: 12,
    marginBottom: 4,
  },
  modelRow: { flexDirection: 'row', alignItems: 'center' },
  modelInfo: { flex: 1 },
  modelName: { fontSize: 15, fontWeight: '500' },
  modelMeta: { fontSize: 12, marginTop: 2 },
  modelCheck: { fontSize: 18, fontWeight: '700', marginLeft: 8 },

  // Attach popover options
  attachOptions: { flex: 1, justifyContent: 'center', padding: 12, gap: 8 },
  attachOption: { flex: 1, justifyContent: 'center', paddingHorizontal: 14 },
  attachOptionText: { fontSize: 15, fontWeight: '500' },

  // Context ring gauge
  gaugeRow: { paddingHorizontal: 4, paddingTop: 6 },
  gaugeRing: { width: 28, height: 28, alignItems: 'center', justifyContent: 'center' },
  gaugeLabel: { position: 'absolute', fontSize: 7, fontWeight: '600', letterSpacing: -0.3 },
});
