import { Globe2, RefreshCcw } from "lucide-react";
import type { NetworkProxySettings, ProxyTestResult } from "../../types";
import type { Copy } from "../../i18n";

export function ProxySettingsPanel({
  copy,
  settings,
  busy,
  result,
  onChange,
  onTest,
  onAutoDetect,
}: {
  copy: Copy;
  settings: NetworkProxySettings;
  busy: "test" | "detect" | null;
  result: ProxyTestResult | null;
  onChange: (value: NetworkProxySettings) => void;
  onTest: () => void;
  onAutoDetect: () => void;
}) {
  const update = (patch: Partial<NetworkProxySettings>) => onChange({ ...settings, ...patch });
  const portValue = settings.port ?? "";
  const statusClass = !settings.enabled ? "muted" : result ? result.ok ? "success" : "error" : "muted";
  const statusText = !settings.enabled
    ? copy.proxyDisabledStatus
    : result
      ? result.ok
        ? `${result.elapsedMs ?? 0} ms${result.httpVersion ? ` · ${result.httpVersion}` : ""}`
        : result.message
      : copy.proxyPendingTest;
  return (
    <>
      <label className="st-row">
        <div className="st-row-copy">
          <strong>{copy.proxyEnable}</strong>
          <span>{copy.proxySettingsDescription}</span>
        </div>
        <span className="st-switch">
          <input
            type="checkbox"
            role="switch"
            aria-checked={settings.enabled}
            checked={settings.enabled}
            onChange={(event) => update({ enabled: event.currentTarget.checked })}
          />
        </span>
      </label>
      <div className="st-block">
        <div className="st-field-grid">
          <label>
            <span>{copy.proxyProtocol}</span>
            <select
              value={settings.scheme}
              onChange={(event) => update({ scheme: event.currentTarget.value as NetworkProxySettings["scheme"] })}
            >
              <option value="http">HTTP</option>
              <option value="https">HTTPS</option>
              <option value="socks5">SOCKS5</option>
              <option value="socks5h">SOCKS5H</option>
            </select>
          </label>
          <label>
            <span>{copy.proxyHost}</span>
            <input
              type="text"
              value={settings.host}
              placeholder="127.0.0.1"
              onChange={(event) => update({ host: event.currentTarget.value })}
            />
          </label>
          <label>
            <span>{copy.proxyPort}</span>
            <input
              type="number"
              min={1}
              max={65535}
              value={portValue}
              placeholder="7890"
              onChange={(event) => {
                const value = event.currentTarget.value.trim();
                update({ port: value ? Number(value) : null });
              }}
            />
          </label>
        </div>
        <div className="st-inline-actions">
          <button className="st-btn" type="button" onClick={onAutoDetect} disabled={busy !== null}>
            <RefreshCcw size={14} />
            {busy === "detect" ? copy.proxyAutoDetecting : copy.proxyAutoDetect}
          </button>
          <button className="st-btn" type="button" onClick={onTest} disabled={busy !== null}>
            <Globe2 size={14} />
            {busy === "test" ? copy.proxyTesting : copy.proxyTest}
          </button>
          <span className={`st-pill st-inline-status ${statusClass}`}>{statusText}</span>
        </div>
      </div>
    </>
  );
}
