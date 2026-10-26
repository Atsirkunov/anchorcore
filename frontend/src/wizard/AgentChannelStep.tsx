import { useState } from "react";
import { McpConnectCard } from "../McpConnect";
import { theme } from "../theme";
import type { LlmMode } from "../types";
import { SkillChannel } from "./SkillChannel";
import { styles } from "./styles";

type Channel = "mcp" | "skill";

const CHANNELS: { id: Channel; title: string; body: string }[] = [
  {
    id: "mcp",
    title: "MCP",
    body: "For agents that can run a local MCP server (Claude Code, Claude Desktop, Cursor, opencode, Codex). One click writes the agent's config.",
  },
  {
    id: "skill",
    title: "REST skill",
    body: "For machines where local MCP servers aren't allowed: a skill file that talks to this app's HTTP API.",
  },
];

/** R45.6: the two equal agent channels — neither is assumed (R45.2). */
export function AgentChannelStep({ mode }: { mode: LlmMode }) {
  const [channel, setChannel] = useState<Channel | null>(null);
  return (
    <div style={styles.body}>
      <div style={styles.note}>
        {mode === "agent"
          ? "Your agent writes the answers, so connect it now. Both ways give it the same memory and citations."
          : "Optional: let an AI agent search this memory too. Both ways give it the same memory and citations."}
      </div>
      <div style={styles.choices} role="radiogroup" aria-label="How your agent connects">
        {CHANNELS.map((c) => (
          <button
            key={c.id}
            role="radio"
            aria-checked={channel === c.id}
            style={{ ...styles.choice, ...(channel === c.id ? styles.choiceActive : {}) }}
            onClick={() => setChannel(c.id)}
          >
            <div style={styles.choiceTitle}>{c.title}</div>
            <div style={{ color: theme.textMuted }}>{c.body}</div>
          </button>
        ))}
      </div>
      {channel === "mcp" && <McpConnectCard />}
      {channel === "skill" && <SkillChannel />}
      {channel && (
        <div style={styles.hint}>
          Use one channel per agent, not both — with MCP tools and the skill active, agents duplicate their work. Change it any time in System.
        </div>
      )}
    </div>
  );
}
