// Ditado por voz no compositor. Usa a Web Speech API do WebView2
// (reconhecimento do Windows/Edge). Clique para começar, clique de
// novo — ou Enter para enviar — para parar.

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

type Recog = {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  onresult: ((ev: RecogEvent) => void) | null;
  onerror: ((ev: { error: string }) => void) | null;
  onend: (() => void) | null;
  start(): void;
  stop(): void;
  abort(): void;
};

type RecogEvent = {
  resultIndex: number;
  results: ArrayLike<{
    isFinal: boolean;
    0: { transcript: string };
  }>;
};

function getCtor(): (new () => Recog) | null {
  const w = window as Window & {
    SpeechRecognition?: new () => Recog;
    webkitSpeechRecognition?: new () => Recog;
  };
  return w.SpeechRecognition ?? w.webkitSpeechRecognition ?? null;
}

function joinParts(...parts: string[]): string {
  return parts
    .map((p) => p.trim())
    .filter(Boolean)
    .join(" ");
}

async function primeMic(): Promise<void> {
  if (!navigator.mediaDevices?.getUserMedia) return;
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  for (const track of stream.getTracks()) track.stop();
}

export default function VoiceButton({
  draft,
  onDraftChange,
  disabled,
}: {
  draft: string;
  onDraftChange: (text: string) => void;
  disabled?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const [listening, setListening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const recRef = useRef<Recog | null>(null);
  const wantRef = useRef(false);
  const baseRef = useRef("");
  const finalsRef = useRef("");
  const draftRef = useRef(draft);
  const onDraftRef = useRef(onDraftChange);
  draftRef.current = draft;
  onDraftRef.current = onDraftChange;

  const supported = !!getCtor();

  const disposeRec = useCallback(() => {
    const rec = recRef.current;
    recRef.current = null;
    if (!rec) return;
    rec.onend = null;
    rec.onresult = null;
    rec.onerror = null;
    try {
      rec.stop();
    } catch {
      /* já parado */
    }
  }, []);

  const stop = useCallback(() => {
    wantRef.current = false;
    disposeRec();
    setListening(false);
  }, [disposeRec]);

  useEffect(() => {
    if (disabled && wantRef.current) stop();
  }, [disabled, stop]);

  useEffect(() => () => stop(), [stop]);

  useEffect(() => {
    if (!error) return;
    const id = window.setTimeout(() => setError(null), 5000);
    return () => window.clearTimeout(id);
  }, [error]);

  const start = useCallback(async () => {
    const Ctor = getCtor();
    if (!Ctor) {
      setError(t("chat.voice.unsupported"));
      return;
    }
    setError(null);
    try {
      await primeMic();
    } catch (e) {
      const name = e instanceof DOMException ? e.name : "";
      if (name === "NotAllowedError" || name === "PermissionDeniedError") {
        setError(t("chat.voice.denied"));
      } else if (name === "NotFoundError") {
        setError(t("chat.voice.noMic"));
      } else {
        setError(t("chat.voice.error"));
      }
      return;
    }

    disposeRec();
    wantRef.current = true;
    baseRef.current = draftRef.current.trimEnd();
    finalsRef.current = "";

    const rec = new Ctor();
    rec.continuous = true;
    rec.interimResults = true;
    rec.lang = i18n.language?.toLowerCase().startsWith("en") ? "en-US" : "pt-BR";

    rec.onresult = (ev) => {
      let interim = "";
      let extraFinal = "";
      for (let i = ev.resultIndex; i < ev.results.length; i++) {
        const r = ev.results[i];
        const text = r[0]?.transcript ?? "";
        if (r.isFinal) extraFinal += text;
        else interim += text;
      }
      if (extraFinal) {
        finalsRef.current = joinParts(finalsRef.current, extraFinal);
      }
      onDraftRef.current(
        joinParts(baseRef.current, finalsRef.current, interim),
      );
    };

    rec.onerror = (ev) => {
      if (ev.error === "no-speech" || ev.error === "aborted") return;
      wantRef.current = false;
      if (ev.error === "not-allowed" || ev.error === "service-not-allowed") {
        setError(t("chat.voice.denied"));
      } else if (ev.error === "audio-capture") {
        setError(t("chat.voice.noMic"));
      } else if (ev.error === "network") {
        setError(t("chat.voice.network"));
      } else {
        setError(t("chat.voice.error"));
      }
    };

    rec.onend = () => {
      if (!wantRef.current) {
        recRef.current = null;
        setListening(false);
        return;
      }
      try {
        rec.start();
      } catch {
        wantRef.current = false;
        recRef.current = null;
        setListening(false);
      }
    };

    recRef.current = rec;
    try {
      rec.start();
      setListening(true);
    } catch {
      wantRef.current = false;
      recRef.current = null;
      setError(t("chat.voice.error"));
    }
  }, [disposeRec, i18n.language, t]);

  const toggle = () => {
    if (listening || wantRef.current) stop();
    else void start();
  };

  return (
    <div className="relative shrink-0">
      <button
        type="button"
        disabled={disabled || !supported}
        onClick={toggle}
        aria-pressed={listening}
        title={
          !supported
            ? t("chat.voice.unsupported")
            : listening
              ? t("chat.voice.stop")
              : t("chat.voice.start")
        }
        className={`relative flex h-8 w-8 items-center justify-center rounded-full transition-colors disabled:opacity-40 ${
          listening
            ? "bg-bad/15 text-bad hover:bg-bad/25"
            : "text-dim hover:bg-panel hover:text-ink"
        }`}
      >
        {listening && (
          <span className="absolute inset-0 animate-ping rounded-full bg-bad/25" />
        )}
        <svg
          className="relative h-[18px] w-[18px]"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <rect x="9" y="2" width="6" height="11" rx="3" />
          <path d="M5 10a7 7 0 0014 0" />
          <path d="M12 17v4M8 21h8" />
        </svg>
      </button>
      {error && (
        <div className="absolute bottom-full left-1/2 z-30 mb-2 w-56 -translate-x-1/2 rounded-lg border border-edge bg-panel px-2.5 py-1.5 text-center text-[11px] text-bad shadow-xl">
          {error}
        </div>
      )}
    </div>
  );
}
