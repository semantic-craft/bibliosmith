import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, KeyRound, Trash2 } from "lucide-react";
import {
  deleteModelCredential,
  getModelCatalog,
  saveModelCredential,
  setActiveModel,
  testModelConnection,
} from "../../api";
import type { ActiveModel, ModelCatalog, ModelSlotView } from "../../types";
import { MODEL_BRANDS, slotKey, slotMeta } from "./modelCatalog";
import { modelsCopy } from "./modelsCopy";

type SlotState = {
  key: string;
  model: string;
  busy: "save" | "test" | "delete" | null;
  message: string | null;
  ok: boolean | null;
};

export function ModelsSettingsPanel({ locale }: { locale: string }) {
  const copy = useMemo(() => modelsCopy(locale), [locale]);
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [slotState, setSlotState] = useState<Record<string, SlotState>>({});

  const refresh = async () => {
    const next = await getModelCatalog();
    setCatalog(next);
  };

  useEffect(() => {
    void refresh();
  }, []);

  const configuredBy = useMemo(() => {
    const map = new Map<string, ModelSlotView>();
    for (const slot of catalog?.slots ?? []) {
      map.set(slotKey(slot.profileId, slot.configId), slot);
    }
    return map;
  }, [catalog]);

  const active: ActiveModel | null = catalog?.active ?? null;

  const stateFor = (key: string, fallbackModel: string): SlotState =>
    slotState[key] ?? {
      key: "",
      model: fallbackModel,
      busy: null,
      message: null,
      ok: null,
    };

  const patch = (key: string, next: Partial<SlotState>) =>
    setSlotState((prev) => ({
      ...prev,
      [key]: { ...stateFor(key, next.model ?? ""), ...prev[key], ...next },
    }));

  const activeMeta = active ? slotMeta(active.profileId, active.configId) : undefined;

  return (
    <div className="st-models">
      <p className="st-models-lead">{copy.description}</p>
      <div className={`st-models-active ${active ? "" : "muted"}`}>
        {active && activeMeta
          ? copy.activeHint(activeMeta.label, active.model)
          : copy.noActive}
      </div>

      {MODEL_BRANDS.map((brand) => (
        <div key={brand.profileId} className="st-models-brand">
          <div className="st-models-brand-head">
            <b>{brand.brand}</b>
            <a href={brand.keyUrl} target="_blank" rel="noreferrer">
              {copy.getKey}
            </a>
          </div>
          {brand.slots.map((meta) => {
            const key = slotKey(meta.profileId, meta.configId);
            const view = configuredBy.get(key);
            const configured = view?.configured ?? false;
            const isActive =
              active?.profileId === meta.profileId &&
              active?.configId === meta.configId;
            const defaultModel = view?.defaultModel ?? meta.models[0];
            const st = stateFor(key, defaultModel);
            const chosenModel = st.model || defaultModel;
            return (
              <div
                key={key}
                className={`st-models-slot${isActive ? " active" : ""}`}
              >
                <div className="st-models-slot-head">
                  <span className="st-models-slot-name">
                    {brand.slots.length > 1 ? meta.label : brand.brand}
                    {isActive && (
                      <span className="st-models-badge">
                        <CheckCircle2 size={13} /> {copy.activeBadge}
                      </span>
                    )}
                  </span>
                  <span
                    className={`st-models-status ${configured ? "ok" : "muted"}`}
                  >
                    {configured ? copy.configured : copy.notConfigured}
                  </span>
                </div>

                <div className="st-models-slot-grid">
                  <label>
                    <span>{copy.model}</span>
                    <select
                      value={chosenModel}
                      onChange={(event) =>
                        patch(key, { model: event.currentTarget.value })
                      }
                    >
                      {Array.from(new Set([defaultModel, ...meta.models])).map(
                        (model) => (
                          <option key={model} value={model}>
                            {model}
                          </option>
                        ),
                      )}
                    </select>
                  </label>
                  <label>
                    <span>{copy.apiKey}</span>
                    <input
                      type="password"
                      autoComplete="off"
                      placeholder={
                        configured ? "••••••••••" : copy.keyPlaceholder
                      }
                      value={st.key}
                      onChange={(event) =>
                        patch(key, { key: event.currentTarget.value, message: null })
                      }
                    />
                  </label>
                </div>

                <div className="st-models-slot-actions">
                  <button
                    className="pl-btn sm"
                    type="button"
                    disabled={st.busy !== null}
                    onClick={async () => {
                      if (!st.key.trim()) {
                        patch(key, { ok: false, message: copy.emptyKey });
                        return;
                      }
                      patch(key, { busy: "save", message: null });
                      try {
                        await saveModelCredential(
                          meta.profileId,
                          meta.configId,
                          st.key.trim(),
                        );
                        patch(key, {
                          busy: null,
                          key: "",
                          ok: true,
                          message: copy.saved,
                        });
                        await refresh();
                      } catch (error) {
                        patch(key, {
                          busy: null,
                          ok: false,
                          message: String(error),
                        });
                      }
                    }}
                  >
                    <KeyRound size={14} />
                    {st.busy === "save" ? copy.saving : copy.save}
                  </button>

                  <button
                    className="pl-btn sm ghost"
                    type="button"
                    disabled={st.busy !== null || (!configured && !st.key.trim())}
                    onClick={async () => {
                      patch(key, { busy: "test", message: null });
                      try {
                        const result = await testModelConnection(
                          meta.profileId,
                          meta.configId,
                          chosenModel,
                          st.key.trim() || undefined,
                        );
                        patch(key, {
                          busy: null,
                          ok: result.ok,
                          message: result.message,
                        });
                      } catch (error) {
                        patch(key, {
                          busy: null,
                          ok: false,
                          message: String(error),
                        });
                      }
                    }}
                  >
                    {st.busy === "test" ? copy.testing : copy.test}
                  </button>

                  <button
                    className="pl-btn sm primary"
                    type="button"
                    disabled={st.busy !== null || !configured || isActive}
                    onClick={async () => {
                      await setActiveModel(
                        meta.profileId,
                        meta.configId,
                        chosenModel,
                      );
                      await refresh();
                    }}
                  >
                    {copy.setActive}
                  </button>

                  {configured && (
                    <button
                      className="pl-btn sm ghost danger"
                      type="button"
                      disabled={st.busy !== null}
                      onClick={async () => {
                        patch(key, { busy: "delete" });
                        try {
                          await deleteModelCredential(
                            meta.profileId,
                            meta.configId,
                          );
                          patch(key, {
                            busy: null,
                            ok: true,
                            message: copy.removed,
                          });
                          await refresh();
                        } catch (error) {
                          patch(key, {
                            busy: null,
                            ok: false,
                            message: String(error),
                          });
                        }
                      }}
                    >
                      <Trash2 size={14} />
                    </button>
                  )}
                </div>

                {st.message && (
                  <div
                    className={`st-models-msg ${
                      st.ok === false ? "error" : st.ok ? "ok" : "muted"
                    }`}
                  >
                    {st.message}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}
