import type {
  DiagnosticLogSettings,
  DownloadProgress,
  LauncherSettings,
  NetworkProxySettings,
  NodeModulesStatus,
  ProxyTestResult,
  RuntimeStatus,
} from "../../types";
import type { Copy, LanguageSetting } from "../../i18n";
import { SettingToggle } from "../../components";
import type { DownloadHudState } from "../../shell";
import { ProjectPathPanel } from "./ProjectPathPanel";
import { ProxySettingsPanel } from "./ProxySettingsPanel";
import { RuntimeSettingsPanel } from "./RuntimeSettingsPanel";
import { NodeModulesSettingsPanel } from "./NodeModulesSettingsPanel";
import { DiagnosticLogPanel } from "./DiagnosticLogPanel";
import { SourceCleanupPanel } from "./SourceCleanupPanel";
import { ModelsSettingsPanel } from "./ModelsSettingsPanel";
import { modelsCopy } from "./modelsCopy";
import { EmbeddingSettingsPanel } from "./EmbeddingSettingsPanel";
import { embeddingCopy } from "./embeddingCopy";
import { OcrSettingsPanel } from "./OcrSettingsPanel";
import { ocrCopy } from "./ocrCopy";
import "./settings.css";

export function SettingsPage({
  copy,
  locale,
  languageSetting,
  settings,
  repoPath,
  proxySettings,
  proxyBusy,
  proxyTestResult,
  runtimeStatus,
  nodeModulesStatus,
  nodeModulesProgress,
  nodeModulesDownloadState,
  nodeModulesMessage,
  diagnosticLogSettings,
  onLanguageChange,
  onUpdateSetting,
  onChooseRepo,
  onProxyChange,
  onProxyTest,
  onProxyAutoDetect,
  onRuntimeRetry,
  onNodeModulesToggle,
  onNodeModulesStop,
  onNodeModulesCancel,
  onExportLogs,
}: {
  copy: Copy;
  locale: string;
  languageSetting: LanguageSetting;
  settings: LauncherSettings;
  repoPath: string;
  proxySettings: NetworkProxySettings;
  proxyBusy: "test" | "detect" | null;
  proxyTestResult: ProxyTestResult | null;
  runtimeStatus: RuntimeStatus | null;
  nodeModulesStatus: NodeModulesStatus | null;
  nodeModulesProgress: DownloadProgress | null;
  nodeModulesDownloadState: DownloadHudState;
  nodeModulesMessage: string;
  diagnosticLogSettings: DiagnosticLogSettings | null;
  onLanguageChange: (value: LanguageSetting) => void;
  onUpdateSetting: (key: keyof LauncherSettings, value: boolean) => void;
  onChooseRepo: () => void;
  onProxyChange: (value: NetworkProxySettings) => void;
  onProxyTest: () => void;
  onProxyAutoDetect: () => void;
  onRuntimeRetry: () => void;
  onNodeModulesToggle: (value: boolean) => void;
  onNodeModulesStop: () => void;
  onNodeModulesCancel: () => void;
  onExportLogs: () => void;
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
          <SettingToggle title={copy.checkLauncherTitle} description={copy.checkLauncherDescription} checked={settings.checkLauncherOnLaunch} onChange={(value) => onUpdateSetting("checkLauncherOnLaunch", value)} />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{copy.settingsGroupProject}</div>
        <div className="st-group-card">
          <ProjectPathPanel copy={copy} path={repoPath} onChange={onChooseRepo} />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{modelsCopy(locale).title}</div>
        <div className="st-group-card">
          <ModelsSettingsPanel locale={locale} />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{embeddingCopy(locale).title}</div>
        <div className="st-group-card">
          <EmbeddingSettingsPanel locale={locale} />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{ocrCopy(locale).title}</div>
        <div className="st-group-card">
          <OcrSettingsPanel locale={locale} />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{copy.proxySettingsTitle}</div>
        <div className="st-group-card">
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
        <div className="st-group-title">{copy.runtimeStatusTitle}</div>
        <div className="st-group-card">
          <RuntimeSettingsPanel
            copy={copy}
            status={runtimeStatus}
            onRetry={onRuntimeRetry}
          />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{copy.settingsGroupDependencies}</div>
        <div className="st-group-card">
          <NodeModulesSettingsPanel
            copy={copy}
            status={nodeModulesStatus}
            progress={nodeModulesProgress}
            state={nodeModulesDownloadState}
            message={nodeModulesMessage}
            onToggle={onNodeModulesToggle}
            onStop={onNodeModulesStop}
            onCancel={onNodeModulesCancel}
          />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{locale.startsWith("zh") ? "源文件清理" : "Source cleanup"}</div>
        <div className="st-group-card">
          <SourceCleanupPanel locale={locale} />
        </div>
      </div>

      <div className="st-group">
        <div className="st-group-title">{copy.settingsGroupDiagnostics}</div>
        <div className="st-group-card">
          <DiagnosticLogPanel
            copy={copy}
            settings={diagnosticLogSettings}
            enabled={settings.saveLogsToLocal}
            onToggle={(value) => onUpdateSetting("saveLogsToLocal", value)}
            onExport={onExportLogs}
          />
        </div>
      </div>
    </section>
  );
}
