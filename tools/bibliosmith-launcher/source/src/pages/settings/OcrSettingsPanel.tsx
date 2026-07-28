import { useEffect, useMemo, useState } from "react";
import { KeyRound, Trash2 } from "lucide-react";
import {
  deleteOcrCredential,
  getOcrCredentialsStatus,
  saveOcrCredential,
  testOcrConnection,
} from "../../api";
import type { OcrCredentialsStatus, OcrServiceStatus } from "../../types";
import { ocrCopy, type OcrCopy } from "./ocrCopy";

type OcrService = "paddleocr" | "mineru";

const KEY_URLS: Record<OcrService, string> = {
  paddleocr: "https://aistudio.baidu.com/account/accessToken",
  mineru: "https://mineru.net/apiManage",
};

function statusLabel(status: OcrServiceStatus | undefined, copy: OcrCopy): string {
  if (!status?.configured) return copy.notConfigured;
  return status.source === "keychain" ? copy.configuredKeychain : copy.configuredEnv;
}

function ServiceCard({
  service,
  name,
  hint,
  status,
  copy,
  onChanged,
}: {
  service: OcrService;
  name: string;
  hint: string;
  status: OcrServiceStatus | undefined;
  copy: OcrCopy;
  onChanged: () => void;
}) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<"save" | "test" | "delete" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [ok, setOk] = useState<boolean | null>(null);
  const keychainConfigured = status?.configured === true && status.source === "keychain";

  return (
    <div className="st-models-slot">
      <div className="st-models-slot-head">
        <span className="st-models-slot-name">{name}</span>
        <span className={`st-models-status ${status?.configured ? "ok" : "muted"}`}>
          {statusLabel(status, copy)}
        </span>
      </div>
      <p className="st-models-lead">{hint}</p>

      <div className="st-models-slot-grid" style={{ gridTemplateColumns: "1fr" }}>
        <label>
          <span>{copy.apiKey}</span>
          <input
            type="password"
            autoComplete="off"
            placeholder={keychainConfigured ? "••••••••••" : copy.keyPlaceholder}
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
              await saveOcrCredential(service, key.trim());
              setKey("");
              setOk(true);
              setMessage(copy.saved);
              onChanged();
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
          disabled={busy !== null || (!keychainConfigured && !key.trim())}
          onClick={async () => {
            setBusy("test");
            setMessage(null);
            try {
              const result = await testOcrConnection(service, key.trim() || undefined);
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

        {keychainConfigured && (
          <button
            className="pl-btn sm ghost danger"
            type="button"
            disabled={busy !== null}
            onClick={async () => {
              setBusy("delete");
              try {
                await deleteOcrCredential(service);
                setOk(true);
                setMessage(copy.removed);
                onChanged();
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
  );
}

export function OcrSettingsPanel({ locale }: { locale: string }) {
  const copy = useMemo(() => ocrCopy(locale), [locale]);
  const [status, setStatus] = useState<OcrCredentialsStatus | null>(null);

  const refresh = async () => {
    const next = await getOcrCredentialsStatus();
    setStatus(next);
  };

  useEffect(() => {
    let cancelled = false;
    void getOcrCredentialsStatus().then((next) => {
      if (!cancelled) setStatus(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="st-models">
      <p className="st-models-lead">{copy.description}</p>

      {(
        [
          ["paddleocr", copy.paddleName, copy.paddleHint],
          ["mineru", copy.mineruName, copy.mineruHint],
        ] as [OcrService, string, string][]
      ).map(([service, name, hint]) => (
        <div key={service} className="st-models-brand">
          <div className="st-models-brand-head">
            <b>{name}</b>
            <a href={KEY_URLS[service]} target="_blank" rel="noreferrer">
              {copy.getKey}
            </a>
          </div>
          <ServiceCard
            service={service}
            name={name}
            hint={hint}
            status={status?.[service]}
            copy={copy}
            onChanged={() => void refresh()}
          />
        </div>
      ))}
    </div>
  );
}
