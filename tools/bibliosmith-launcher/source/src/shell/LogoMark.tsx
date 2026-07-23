import launcherIconUrl from "../../assets/bibliosmith-launcher-icon.png";

export function LogoMark({ large }: { large?: boolean }) {
  return (
    <div className={large ? "logo-mark large" : "logo-mark"}>
      <img src={launcherIconUrl} alt="" />
    </div>
  );
}
