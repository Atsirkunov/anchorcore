import { McpConnectCard } from "../McpConnect";
import { theme } from "../theme";
import { AgentSkillsCard } from "./agents/AgentSkillsCard";
import { SensitiveSharingCard } from "./SensitiveSharingCard";

/** R49.9: the Agents tab — the "your agent writes it" path in one place:
 *  connect a coding agent over MCP, the skill files for when MCP is blocked,
 *  and whether agents may read Sensitive content (the same audited switch as
 *  Settings → Sensitive sharing). */
export function AgentsTab() {
  return (
    <div>
      <h2 style={{ marginBottom: 4 }}>Agents</h2>
      <p style={{ margin: "0 0 20px", color: theme.textMuted, fontSize: 14 }}>
        Let a coding agent search your library and cite it. AnchorCore stays read-only to agents.
      </p>
      <McpConnectCard />
      <div style={{ marginBottom: 16 }}>
        <SensitiveSharingCard refreshKey={0} only={["agents"]} />
      </div>
      <AgentSkillsCard />
    </div>
  );
}
