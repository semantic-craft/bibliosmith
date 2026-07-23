import type { ActivityItem } from "../../types";
import type { Copy } from "../../i18n";
import "./logs.css";

const LEVEL_LABELS: Record<ActivityItem["level"], string> = {
  info: "INFO",
  success: "OK",
  warning: "WARN",
  error: "ERROR",
};

export function LogsPage({ copy, activities }: { copy: Copy; activities: ActivityItem[] }) {
  return (
    <section className="lg-panel">
      <header className="lg-head">
        <h1>{copy.recentActivity}</h1>
        <span className="lg-count">{copy.logEntryCount(activities.length)}</span>
      </header>
      <div className="lg-scroll">
        {activities.length ? (
          activities.map((item) => (
            <div className="lg-row" key={item.id}>
              <span className="lg-time">{item.time}</span>
              <span className={`lg-level ${item.level}`}>{LEVEL_LABELS[item.level]}</span>
              <span className="lg-message">{item.message}</span>
            </div>
          ))
        ) : (
          <div className="lg-empty">{copy.logEmpty}</div>
        )}
      </div>
    </section>
  );
}
