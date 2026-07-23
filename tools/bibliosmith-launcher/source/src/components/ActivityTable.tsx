import { Clock3 } from "lucide-react";
import type { ActivityItem } from "../types";
import type { Copy } from "../i18n";
import { PanelHeading } from "./PanelHeading";

export function ActivityTable({ copy, activities, expanded, onViewFullLog }: { copy: Copy; activities: ActivityItem[]; expanded?: boolean; onViewFullLog: () => void }) {
  return (
    <section className={`data-panel activity-panel ${expanded ? "expanded" : ""}`}>
      <div className="panel-title-row">
        <PanelHeading title={copy.recentActivity} />
        {!expanded && <button className="panel-button" onClick={onViewFullLog}>{copy.viewFullLog}</button>}
      </div>
      <div className="table-wrap activity-table-wrap">
        <table className="data-table activity-table">
          <tbody>
            {activities.map((item) => (
              <tr key={item.id}>
                <td><Clock3 size={16} />{item.time}</td>
                <td>{item.message}</td>
                <td><span className={`level-badge ${item.level}`}>{copy.info}</span></td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
