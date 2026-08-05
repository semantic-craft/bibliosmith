import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, KeyRound, Trash2 } from "lucide-react";
import {
  deleteModelCredential,
  getModelCatalog,
  saveModelCredential,
  saveQwenSettings,
  setActiveModel,
  testModelConnection,
} from "../../api";
import type { ActiveModel, ModelCatalog, ModelSlotView } from "../../types";
import {
  MODEL_BRANDS,
  slotDisplayName,
  slotKey,
  slotMeta,
} from "./modelCatalog";
import { modelsCopy } from "./modelsCopy";

type SlotState = {
  key: string;
  model: string;
  workspaceId: string;
  webSearchEnabled: boolean;
  busy: "save" | "qwen" | "test" | "delete" | null;
  message: string | null;
  ok: boolean | null;
};

const MODEL_SLOT_KEYS = new Set(
  MODEL_BRANDS.flatMap((brand) =>
    brand.slots.map((meta) => slotKey(meta.profileId, meta.configId)),
  ),
);

const FIRST_MODEL_SLOT_KEY = slotKey(
  MODEL_BRANDS[0].slots[0].profileId,
  MODEL_BRANDS[0].slots[0].configId,
);

export function ModelsSettingsPanel({ locale }: { locale: string }) {
  const copy = useMemo(() => modelsCopy(locale), [locale]);
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [slotState, setSlotState] = useState<Record<string, SlotState>>({});
  const [selectedSlotKey, setSelectedSlotKey] = useState<string | null>(null);

  const refresh = async () => {
    const next = await getModelCatalog();
    setCatalog(next);
  };

  useEffect(() => {
    let cancelled = false;
    void getModelCatalog().then((next) => {
      if (!cancelled) setCatalog(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const configuredBy = useMemo(() => {
    const map = new Map<string, ModelSlotView>();
    for (const slot of catalog?.slots ?? []) {
      map.set(slotKey(slot.profileId, slot.configId), slot);
    }
    return map;
  }, [catalog]);

  const active: ActiveModel | null = catalog?.active ?? null;
  const candidateActiveSlotKey = active
    ? slotKey(active.profileId, active.configId)
    : null;
  const activeSlotKey =
    candidateActiveSlotKey && MODEL_SLOT_KEYS.has(candidateActiveSlotKey)
      ? candidateActiveSlotKey
      : null;
  const firstConfiguredSlot = catalog?.slots.find(
    (slot) =>
      slot.configured &&
      MODEL_SLOT_KEYS.has(slotKey(slot.profileId, slot.configId)),
  );
  const firstConfiguredSlotKey = firstConfiguredSlot
    ? slotKey(firstConfiguredSlot.profileId, firstConfiguredSlot.configId)
    : null;
  const visibleSlotKey =
    selectedSlotKey ??
    activeSlotKey ??
    firstConfiguredSlotKey ??
    FIRST_MODEL_SLOT_KEY;

  const stateFor = (
    key: string,
    fallbackModel: string,
    fallbackWorkspaceId = "",
    fallbackWebSearchEnabled = false,
  ): SlotState =>
    slotState[key] ?? {
      key: "",
      model: fallbackModel,
      workspaceId: fallbackWorkspaceId,
      webSearchEnabled: fallbackWebSearchEnabled,
      busy: null,
      message: null,
      ok: null,
    };

  const patch = (key: string, next: Partial<SlotState>) =>
    setSlotState((prev) => ({
      ...prev,
      [key]: {
        ...stateFor(
          key,
          configuredBy.get(key)?.defaultModel ??
            MODEL_BRANDS.flatMap((brand) => brand.slots).find(
              (slot) => slotKey(slot.profileId, slot.configId) === key,
            )?.models[0] ??
            "",
          configuredBy.get(key)?.workspaceId ?? "",
          configuredBy.get(key)?.webSearchEnabled ?? false,
        ),
        ...prev[key],
        ...next,
      },
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

      <div className="st-models-picker st-row">
        <div className="st-row-copy">
          <strong>{copy.modelPicker}</strong>
          <span>{copy.modelPickerDescription}</span>
        </div>
        <select
          className="st-select"
          aria-label={copy.modelPicker}
          value={visibleSlotKey}
          onChange={(event) => setSelectedSlotKey(event.currentTarget.value)}
        >
          {MODEL_BRANDS.map((brand) =>
            brand.slots.map((meta) => (
              <option
                key={slotKey(meta.profileId, meta.configId)}
                value={slotKey(meta.profileId, meta.configId)}
              >
                {slotDisplayName(meta.profileId, meta.configId)}
              </option>
            )),
          )}
        </select>
      </div>

      {MODEL_BRANDS.filter((brand) =>
        brand.slots.some(
          (meta) => slotKey(meta.profileId, meta.configId) === visibleSlotKey,
        ),
      ).map((brand) => (
        <div key={brand.profileId} className="st-models-brand">
          <div className="st-models-brand-head">
            <b>{brand.brand}</b>
            <a href={brand.keyUrl} target="_blank" rel="noreferrer">
              {copy.getKey}
            </a>
          </div>
          {brand.slots.filter(
            (meta) => slotKey(meta.profileId, meta.configId) === visibleSlotKey,
          ).map((meta) => {
            const key = slotKey(meta.profileId, meta.configId);
            const view = configuredBy.get(key);
            const configured = view?.configured ?? false;
            const isActive =
              active?.profileId === meta.profileId &&
              active?.configId === meta.configId;
            const defaultModel = view?.defaultModel ?? meta.models[0];
            const st = stateFor(
              key,
              defaultModel,
              view?.workspaceId ?? "",
              view?.webSearchEnabled ?? false,
            );
            const chosenModel = st.model;
            return (
              <div
                key={key}
                className={`st-models-slot${isActive ? " active" : ""}`}
                role="group"
                aria-label={slotDisplayName(meta.profileId, meta.configId)}
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
                    {meta.allowCustomModel ? (
                      <>
                        <input
                          list={`${key}-models`}
                          value={chosenModel}
                          onChange={(event) =>
                            patch(key, { model: event.currentTarget.value })
                          }
                        />
                        <datalist id={`${key}-models`}>
                          {Array.from(new Set([defaultModel, ...meta.models])).map(
                            (model) => (
                              <option key={model} value={model} />
                            ),
                          )}
                        </datalist>
                      </>
                    ) : (
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
                    )}
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
                  {meta.allowWorkspaceId && (
                    <label>
                      <span>{copy.workspaceId}</span>
                      <input
                        value={st.workspaceId}
                        placeholder={copy.workspacePlaceholder}
                        onChange={(event) =>
                          patch(key, {
                            workspaceId: event.currentTarget.value,
                            message: null,
                          })
                        }
                      />
                    </label>
                  )}
                </div>

                {meta.allowWorkspaceId && (
                  <label className="st-row">
                    <div className="st-row-copy">
                      <strong>{copy.webSearch}</strong>
                      <span>{copy.webSearchDescription}</span>
                    </div>
                    <span className="st-switch">
                      <input
                        type="checkbox"
                        role="switch"
                        aria-label={copy.webSearch}
                        aria-checked={st.webSearchEnabled}
                        checked={st.webSearchEnabled}
                        onChange={(event) =>
                          patch(key, {
                            webSearchEnabled: event.currentTarget.checked,
                            message: null,
                          })
                        }
                      />
                    </span>
                  </label>
                )}

                <div className="st-models-slot-actions">
                  {meta.allowWorkspaceId && (
                    <button
                      className="pl-btn sm ghost"
                      type="button"
                      disabled={st.busy !== null}
                      onClick={async () => {
                        patch(key, { busy: "qwen", message: null });
                        try {
                          await saveQwenSettings(
                            st.workspaceId.trim(),
                            st.webSearchEnabled,
                          );
                          patch(key, {
                            busy: null,
                            ok: true,
                            message: copy.qwenSettingsSaved,
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
                      {st.busy === "qwen"
                        ? copy.savingQwenSettings
                        : copy.saveQwenSettings}
                    </button>
                  )}
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
