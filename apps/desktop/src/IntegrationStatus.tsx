import type { ReactNode } from "react";
import { AlertCircle, CheckCircle2, CircleMinus } from "lucide-react";

export function IntegrationStatus({ state, children }: {
  state: "active" | "inactive" | "warning";
  children: ReactNode;
}) {
  const Icon = state === "active" ? CheckCircle2 : state === "warning" ? AlertCircle : CircleMinus;
  return <span className={`integration-status ${state}`}>
    <Icon size={13} aria-hidden="true" />
    {children}
  </span>;
}
