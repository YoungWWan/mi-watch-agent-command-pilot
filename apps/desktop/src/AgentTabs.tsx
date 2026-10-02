import { useRef } from "react";

const sections = [
  { key: "mcp", label: "MCP 选择题" },
  { key: "hooks", label: "审批与完成通知" },
  { key: "notifications", label: "手表唤醒通知" },
  { key: "rules", label: "信任规则" },
] as const;

export type AgentSection = (typeof sections)[number]["key"];

export function AgentTabs({ activeSection, onChange }: {
  activeSection: AgentSection;
  onChange: (section: AgentSection) => void;
}) {
  const tabs = useRef<Array<HTMLButtonElement | null>>([]);

  return (
    <div className="agent-tabs-bar">
      <div className="agent-tabs" role="tablist" aria-label="Agent 设置分类">
        {sections.map((section, index) => (
          <button
            key={section.key}
            ref={element => { tabs.current[index] = element; }}
            type="button"
            role="tab"
            id={`agent-tab-${section.key}`}
            aria-selected={activeSection === section.key}
            aria-controls={`agent-panel-${section.key}`}
            tabIndex={activeSection === section.key ? 0 : -1}
            onClick={() => onChange(section.key)}
            onKeyDown={event => {
              let nextIndex: number;
              switch (event.key) {
                case "ArrowRight": nextIndex = (index + 1) % sections.length; break;
                case "ArrowLeft": nextIndex = (index + sections.length - 1) % sections.length; break;
                case "Home": nextIndex = 0; break;
                case "End": nextIndex = sections.length - 1; break;
                default: return;
              }
              event.preventDefault();
              tabs.current[nextIndex]?.focus();
              onChange(sections[nextIndex].key);
            }}
          >
            {section.label}
          </button>
        ))}
      </div>
    </div>
  );
}
