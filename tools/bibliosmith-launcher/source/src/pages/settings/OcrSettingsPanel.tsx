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
  return copy.configuredKeychain;
}

function ServiceCard({
  service,
  name,
  hint,
  status,
  draftKey,
  copy,
  onDraftKeyChange,
  onChanged,
}: {
  service: OcrService;
  name: string;
  hint: string;
  status: OcrServiceStatus | undefined;
  draftKey: string;
  copy: OcrCopy;
  onDraftKeyChange: (value: string) => void;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState<"save" | "test" | "delete" | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [ok, setOk] = useState<boolean | null>(null);
  const keychainConfigured = status?.configured === true && status.source === "keychain";

  return (
    <div className="st-models-slot" role="group" aria-label={name}>
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
            value={draftKey}
            onChange={(event) => {
              onDraftKeyChange(event.currentTarget.value);
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
            if (!draftKey.trim()) {
              setOk(false);
              setMessage(copy.emptyKey);
              return;
            }
            setBusy("save");
            setMessage(null);
            try {
              await saveOcrCredential(service, draftKey.trim());
              onDraftKeyChange("");
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
          disabled={busy !== null || (!keychainConfigured && !draftKey.trim())}
          onClick={async () => {
            setBusy("test");
            setMessage(null);
            try {
              const result = await testOcrConnection(
                service,
                draftKey.trim() || undefined,
              );
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
  const [selectedService, setSelectedService] = useState<OcrService | null>(null);
  const [draftKeys, setDraftKeys] = useState<Record<OcrService, string>>({
    paddleocr: "",
    mineru: "",
  });

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

  const services: [OcrService, string, string][] = [
    ["paddleocr", copy.paddleName, copy.paddleHint],
    ["mineru", copy.mineruName, copy.mineruHint],
  ];
  const firstConfiguredService = services.find(
    ([service]) => status?.[service].configured,
  )?.[0];
  const visibleService =
    selectedService ?? firstConfiguredService ?? services[0][0];

  return (
    <div className="st-models">
      <p className="st-models-lead">{copy.description}</p>

      <div className="st-models-picker st-row">
        <div className="st-row-copy">
          <strong>{copy.modelPicker}</strong>
          <span>{copy.modelPickerDescription}</span>
        </div>
        <select
          className="st-select"
          aria-label={copy.modelPicker}
          value={visibleService}
          onChange={(event) =>
            setSelectedService(event.currentTarget.value as OcrService)
          }
        >
          {services.map(([service, name]) => (
            <option key={service} value={service}>
              {name}
            </option>
          ))}
        </select>
      </div>

      {services.filter(([service]) => service === visibleService).map(([service, name, hint]) => (
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
            draftKey={draftKeys[service]}
            copy={copy}
            onDraftKeyChange={(value) =>
              setDraftKeys((current) => ({ ...current, [service]: value }))
            }
            onChanged={() => void refresh()}
          />
        </div>
      ))}
    </div>
  );
}
