import type { LauncherSettings, NetworkProxySettings, ProxyTestResult } from "../../types";
import type { Copy, LanguageSetting } from "../../i18n";
import { SettingToggle } from "../../components";
import { ProxySettingsPanel } from "./ProxySettingsPanel";
import { ModelsSettingsPanel } from "./ModelsSettingsPanel";
import { modelsCopy } from "./modelsCopy";
import { OcrSettingsPanel } from "./OcrSettingsPanel";
import { ocrCopy } from "./ocrCopy";
import "./settings.css";

export function SettingsPage({
  copy,
  locale,
  languageSetting,
  settings,
  proxySettings,
  proxyBusy,
  proxyTestResult,
  onLanguageChange,
  onUpdateSetting,
  onProxyChange,
  onProxyTest,
  onProxyAutoDetect,
}: {
  copy: Copy;
  locale: string;
  languageSetting: LanguageSetting;
  settings: LauncherSettings;
  proxySettings: NetworkProxySettings;
  proxyBusy: "test" | "detect" | null;
  proxyTestResult: ProxyTestResult | null;
  onLanguageChange: (value: LanguageSetting) => void;
  onUpdateSetting: (key: keyof LauncherSettings, value: boolean) => void;
  onProxyChange: (value: NetworkProxySettings) => void;
  onProxyTest: () => void;
  onProxyAutoDetect: () => void;
}) {
  return (
    <section className="st-page">
      <h1 className="st-title">{copy.settingsTitle}</h1>

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
          <ProxySettingsPanel
            copy={copy}
            settings={proxySettings}
            busy={proxyBusy}
            result={proxyTestResult}
            onChange={onProxyChange}
            onTest={onProxyTest}
            onAutoDetect={onProxyAutoDetect}
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
    </section>
  );
}
