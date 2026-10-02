import { Bluetooth, Check, Watch } from "lucide-react";

// An abstract device silhouette; the display communicates connection state only.
export function WatchIllustration({
  connected = false,
  compact = false,
}: {
  connected?: boolean;
  compact?: boolean;
}) {
  return (
    <div
      className={`watch-illustration${compact ? " compact" : ""}${connected ? " connected" : ""}`}
      aria-hidden="true"
    >
      <div className="watch-strap strap-top" />
      <div className="watch-strap strap-bottom" />
      <div className="watch-crown" />
      <div className="watch-case">
        <div className="watch-screen">
          <svg className="watch-dial" viewBox="0 0 144 174" fill="none">
            <rect
              x="11"
              y="11"
              width="122"
              height="152"
              rx="34"
              stroke="currentColor"
              strokeWidth="1"
            />
          </svg>
          <div className="watch-screen-content">
            {connected ? (
              <Check size={28} strokeWidth={1.5} />
            ) : compact ? (
              <Watch size={28} strokeWidth={1.5} />
            ) : (
              <Bluetooth size={28} strokeWidth={1.5} />
            )}
            {compact && <span>指令助手</span>}
          </div>
        </div>
      </div>
    </div>
  );
}
