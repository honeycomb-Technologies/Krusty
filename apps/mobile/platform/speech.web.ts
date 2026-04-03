import { useEffect, useRef } from 'react';

const SpeechRecognition = typeof window !== 'undefined'
  ? (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition
  : null;

let recognition: any = null;
const listeners = new Map<string, Set<(event: any) => void>>();

function getRecognition() {
  if (!recognition && SpeechRecognition) {
    recognition = new SpeechRecognition();
    recognition.continuous = false;
    recognition.interimResults = false;
    recognition.lang = 'en-US';

    recognition.onresult = (e: any) => {
      const transcript = e.results[e.resultIndex]?.[0]?.transcript ?? '';
      emit('result', { isFinal: true, results: [{ transcript }] });
    };
    recognition.onend = () => emit('end', {});
    recognition.onerror = (e: any) => emit('error', { error: e.error });
    recognition.onaudioprocess = (e: any) => {
      emit('volumechange', { value: e.volume ?? 0 });
    };
  }
  return recognition;
}

function emit(event: string, data: any) {
  listeners.get(event)?.forEach(cb => cb(data));
}

export const ExpoSpeechRecognitionModule = SpeechRecognition ? {
  async requestPermissionsAsync() {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      stream.getTracks().forEach(t => t.stop());
      return { granted: true };
    } catch {
      return { granted: false };
    }
  },
  start(options?: { lang?: string; interimResults?: boolean; continuous?: boolean }) {
    const rec = getRecognition();
    if (!rec) return;
    if (options?.lang) rec.lang = options.lang;
    if (options?.interimResults !== undefined) rec.interimResults = options.interimResults;
    if (options?.continuous !== undefined) rec.continuous = options.continuous;
    rec.start();
  },
  stop() {
    getRecognition()?.stop();
  },
} : null;

export function useSpeechRecognitionEvent(event: string, callback: (data: any) => void) {
  const cbRef = useRef(callback);
  cbRef.current = callback;

  useEffect(() => {
    const handler = (data: any) => cbRef.current(data);
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event)!.add(handler);
    return () => { listeners.get(event)?.delete(handler); };
  }, [event]);
}
