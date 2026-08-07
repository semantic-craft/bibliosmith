import { useEffect } from "react";
import { X } from "lucide-react";
import type {
  LauncherSettings,
  TranslationPromptPackCatalog,
  TranslationPromptPackReference,
  TranslationPromptPackRevisionDraft,
} from "../../types";
import type { Copy, LanguageSetting } from "../../i18n";
import { SettingToggle } from "../../components";
import { ModelsSettingsPanel } from "./ModelsSettingsPanel";
import { modelsCopy } from "./modelsCopy";
import { OcrSettingsPanel } from "./OcrSettingsPanel";
import { ocrCopy } from "./ocrCopy";
import { PromptPackSettingsPanel } from "./PromptPackSettingsPanel";
import "./settings.css";

/**
 * Settings is an overlay, not a page: it floats over the shelf rather than
 * replacing it, so leaving it never costs the reader their place. The gear
 * toggles it, Escape and the scrim close it.
 */
export function SettingsOverlay({
  copy,
  locale,
  languageSetting,
  settings,
  onLanguageChange,
  onUpdateSetting,
  promptPackCatalog,
  promptPackDefaults,
  promptPackBusy,
  onCopyPromptPack,
  onSavePromptPackRevision,
  onDeletePromptPack,
  onSetPromptPackDefault,
  onClose,
}: {
  copy: Copy;
  locale: string;
  languageSetting: LanguageSetting;
  settings: LauncherSettings;
  onLanguageChange: (value: LanguageSetting) => void;
  onUpdateSetting: (key: keyof LauncherSettings, value: boolean) => void;
  promptPackCatalog: TranslationPromptPackCatalog | null;
  promptPackDefaults: Record<"programmatic" | "expert-agent", TranslationPromptPackReference | null>;
  promptPackBusy: boolean;
  onCopyPromptPack: (source: TranslationPromptPackReference, displayName: string) => Promise<void>;
  onSavePromptPackRevision: (draft: TranslationPromptPackRevisionDraft) => Promise<void>;
  onDeletePromptPack: (packId: string) => Promise<void>;
  onSetPromptPackDefault: (executor: "programmatic" | "expert-agent", value: TranslationPromptPackReference) => Promise<void>;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="st-overlay">
      <div className="st-scrim" onClick={onClose} aria-hidden />
      <section className="st-panel" role="dialog" aria-modal="true" aria-label={copy.settingsTitle}>
        <header className="st-panel-head">
          <h1 className="st-title">{copy.settingsTitle}</h1>
          <button className="st-close" type="button" aria-label={copy.close} title={copy.close} onClick={onClose}>
            <X size={16} />
          </button>
        </header>

        <div className="st-panel-body">
          <div className="st-group">
            <div className="st-group-title">{copy.settingsGroupGeneral}</div>
            <div className="st-group-card">
              <div className="st-row">
                <div className="st-row-copy">
                  <strong>{copy.languageTitle}</strong>
                  <span>{copy.languageDescription}</span>
                </div>
                <select
                  className="st-select"
                  value={languageSetting}
                  onChange={(event) => onLanguageChange(event.currentTarget.value as LanguageSetting)}
                >
                  <option value="system">{copy.languageSystem}</option>
                  <option value="zh-CN">中文（简体）</option>
                  <option value="zh-TW">中文（繁體）</option>
                  <option value="ja">日本語</option>
                  <option value="en">English</option>
                </select>
              </div>
              <SettingToggle title={copy.autoStartTitle} description={copy.autoStartDescription} checked={settings.autoStart} onChange={(value) => onUpdateSetting("autoStart", value)} />
            </div>
          </div>

          <div className="st-group">
            <div className="st-group-card">
              <PromptPackSettingsPanel
                locale={locale}
                catalog={promptPackCatalog}
                defaults={promptPackDefaults}
                busy={promptPackBusy}
                onCopy={onCopyPromptPack}
                onSaveRevision={onSavePromptPackRevision}
                onDelete={onDeletePromptPack}
                onSetDefault={onSetPromptPackDefault}
              />
            </div>
          </div>

          <div className="st-group">
            <div className="st-group-title">{modelsCopy(locale).title}</div>
            <div className="st-group-card">
              <ModelsSettingsPanel locale={locale} />
            </div>
          </div>

          <div className="st-group">
            <div className="st-group-title">{ocrCopy(locale).title}</div>
            <div className="st-group-card">
              <OcrSettingsPanel locale={locale} />
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
