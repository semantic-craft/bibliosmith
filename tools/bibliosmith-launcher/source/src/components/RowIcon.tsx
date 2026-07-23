import { CalendarDays, FileText, RefreshCcw, Settings } from "lucide-react";

export function RowIcon({ index }: { index: number }) {
  const icons = [CalendarDays, RefreshCcw, FileText, Settings];
  const Icon = icons[index % icons.length];
  return <Icon size={16} className={`row-icon color-${index % 4}`} />;
}
