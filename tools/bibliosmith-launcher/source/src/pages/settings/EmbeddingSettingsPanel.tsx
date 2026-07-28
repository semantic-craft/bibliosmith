import { useEffect, useMemo, useState } from "react";
import { KeyRound, Trash2 } from "lucide-react";
import {
  deleteEmbeddingCredential,
  getEmbeddingStatus,
  saveEmbeddingCredential,
  testEmbeddingConnection,
} from "../../api";
import type { EmbeddingStatus } from "../../types";
import { embeddingCopy } from "./embeddingCopy";

const GEMINI_KEY_URL = "https://aistudio.google.com/apikey";

export function EmbeddingSettingsPanel({ locale }: { locale: string }) {
  const copy = useMemo(() => embeddingCopy(locale), [locale]);
  const [status, setStatus] = useState<EmbeddingStatus | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<"save" | "test" | "delete" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [ok, setOk] = useState<boolean | null>(null);

  const refresh = async () => {
    const next = await getEmbeddingStatus();
    setStatus(next);
  };

  useEffect(() => {
    let cancelled = false;
    void getEmbeddingStatus().then((next) => {
      if (!cancelled) setStatus(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const configured = status?.configured ?? false;

  return (
    <div className="st-models">
      <p className="st-models-lead">{copy.description}</p>
      <div className="st-models-active muted">{copy.lockedHint}</div>

      <div className="st-models-brand">
        <div className="st-models-brand-head">
          <b>Gemini</b>
          <a href={GEMINI_KEY_URL} target="_blank" rel="noreferrer">
            {copy.getKey}
          </a>
        </div>
        <div className="st-models-slot">
          <div className="st-models-slot-head">
            <span className="st-models-slot-name">Gemini Embedding</span>
            <span className={`st-models-status ${configured ? "ok" : "muted"}`}>
              {configured ? copy.configured : copy.notConfigured}
            </span>
          </div>

          <div className="st-models-slot-grid" style={{ gridTemplateColumns: "1fr" }}>
            <label>
              <span>{copy.apiKey}</span>
              <input
                type="password"
                autoComplete="off"
                placeholder={configured ? "••••••••••" : copy.keyPlaceholder}
                value={key}
                onChange={(event) => {
                  setKey(event.currentTarget.value);
                  setMessage(null);
                }}
              />
            </label>
          </div>

          <div className="st-models-slot-actions">
            <button
              className="pl-btn sm"
              type="button"
              disabled={busy !== null}
              onClick={async () => {
                if (!key.trim()) {
                  setOk(false);
                  setMessage(copy.emptyKey);
                  return;
                }
                setBusy("save");
                setMessage(null);
                try {
                  await saveEmbeddingCredential(key.trim());
                  setKey("");
                  setOk(true);
                  setMessage(copy.saved);
                  await refresh();
                } catch (error) {
                  setOk(false);
                  setMessage(String(error));
                } finally {
                  setBusy(null);
                }
              }}
            >
              <KeyRound size={14} />
              {busy === "save" ? copy.saving : copy.save}
            </button>

            <button
              className="pl-btn sm ghost"
              type="button"
              disabled={busy !== null || (!configured && !key.trim())}
              onClick={async () => {
                setBusy("test");
                setMessage(null);
                try {
                  const result = await testEmbeddingConnection(key.trim() || undefined);
                  setOk(result.ok);
                  setMessage(result.message);
                } catch (error) {
                  setOk(false);
                  setMessage(String(error));
                } finally {
                  setBusy(null);
                }
              }}
            >
              {busy === "test" ? copy.testing : copy.test}
            </button>

            {configured && (
              <button
                className="pl-btn sm ghost danger"
                type="button"
                disabled={busy !== null}
                onClick={async () => {
                  setBusy("delete");
                  try {
                    await deleteEmbeddingCredential();
                    setOk(true);
                    setMessage(copy.removed);
                    await refresh();
                  } catch (error) {
                    setOk(false);
                    setMessage(String(error));
                  } finally {
                    setBusy(null);
                  }
                }}
              >
                <Trash2 size={14} />
              </button>
            )}
          </div>

          {message && (
            <div className={`st-models-msg ${ok === false ? "error" : ok ? "ok" : "muted"}`}>
              {message}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
