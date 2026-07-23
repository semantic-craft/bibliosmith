export function SettingToggle(props: { title: string; description: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="st-row">
      <div className="st-row-copy">
        <strong>{props.title}</strong>
        <span>{props.description}</span>
      </div>
      <span className="st-switch">
        <input
          type="checkbox"
          role="switch"
          aria-checked={props.checked}
          checked={props.checked}
          onChange={(event) => props.onChange(event.target.checked)}
        />
      </span>
    </label>
  );
}
