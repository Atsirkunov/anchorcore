import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { McpConnectCard } from "./McpConnect";
import type { McpClient, McpInfo } from "./types";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const MCP: McpInfo = { mode: "stdio", command: "/app/anchorcore", args: ["mcp"], calls: 2, last_call_at: null };

function client(over: Partial<McpClient> & { id: string; name: string }): McpClient {
  return { detected: true, connected: false, method: "file", config_path: null, ...over };
}

const CLIENTS = [
  client({ id: "claude-code", name: "Claude Code", connected: true, method: "cli" }),
  client({ id: "cursor", name: "Cursor", config_path: "/home/u/.cursor/mcp.json" }),
  client({ id: "claude-desktop", name: "Claude Desktop", detected: false, config_path: "/home/u/.config/Claude/claude_desktop_config.json" }),
];

let root: Root | null = null;
let container: HTMLDivElement | null = null;

async function renderCard() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(<McpConnectCard />);
  });
  await act(async () => {});
  return container;
}

afterEach(async () => {
  vi.restoreAllMocks();
  if (root) {
    await act(async () => {
      root!.unmount();
    });
    root = null;
  }
  container?.remove();
  container = null;
});

function mockApi() {
  vi.spyOn(api, "systemMcp").mockResolvedValue(MCP);
  vi.spyOn(api, "mcpClients").mockResolvedValue({ clients: CLIENTS });
}

describe("McpConnectCard", () => {
  it("lists clients with connected/detected/not-detected states", async () => {
    mockApi();
    const host = await renderCard();
    expect(host.textContent).toContain("Claude Code");
    expect(host.textContent).toContain("connected");
    expect(host.textContent).toContain("Cursor");
    expect(host.textContent).toContain("detected");
    expect(host.textContent).toContain("Claude Desktop");
    expect(host.textContent).toContain("not detected");
    expect(host.textContent).toContain("2 calls audited");
  });

  it("renders the exact CLI line and JSON snippets for copy-paste setup", async () => {
    mockApi();
    const host = await renderCard();
    const code = host.querySelector("code");
    expect(code?.textContent).toBe('claude mcp add anchorcore -- "/app/anchorcore" mcp');
    const pres = host.querySelectorAll("pre");
    expect(pres.length).toBe(2);
    const desktop = JSON.parse(pres[0].textContent ?? "{}");
    expect(desktop.mcpServers.anchorcore.command).toBe("/app/anchorcore");
    expect(desktop.mcpServers.anchorcore.args).toEqual(["mcp"]);
    const opencode = JSON.parse(pres[1].textContent ?? "{}");
    expect(opencode.mcp.servers.anchorcore.type).toBe("local");
    expect(opencode.mcp.servers.anchorcore.command).toEqual(["/app/anchorcore", "mcp"]);
  });

  it("adds ANCHOR_BACKEND_URL env only for non-default backends", async () => {
    mockApi();
    const withEnv = await renderCard();
    const desktop = JSON.parse(withEnv.querySelectorAll("pre")[0].textContent ?? "{}");
    expect(desktop.mcpServers.anchorcore.env).toEqual({ ANCHOR_BACKEND_URL: window.location.origin });

    // default local backend: no env needed
    const locDesc = Object.getOwnPropertyDescriptor(window, "location")!;
    Object.defineProperty(window, "location", { value: { origin: "http://127.0.0.1:8000" }, configurable: true });
    try {
      await act(async () => {
        root!.unmount();
      });
      root = null;
      container?.remove();
      container = document.createElement("div");
      document.body.appendChild(container);
      root = createRoot(container);
      await act(async () => {
        root!.render(<McpConnectCard />);
      });
      await act(async () => {});
      const plain = JSON.parse(container.querySelectorAll("pre")[0].textContent ?? "{}");
      expect(plain.mcpServers.anchorcore.env).toBeUndefined();
    } finally {
      Object.defineProperty(window, "location", locDesc);
    }
  });

  it("connects a detected client and reports where it wrote", async () => {
    mockApi();
    const connect = vi.spyOn(api, "mcpConnect").mockResolvedValue({ ok: true, method: "file", config_path: "/home/u/.cursor/mcp.json" });
    const host = await renderCard();
    const buttons = Array.from(host.querySelectorAll("button"));
    // order: Disconnect (connected claude-code), Connect (cursor), Connect disabled (desktop)
    expect(buttons[0].textContent).toBe("Disconnect");
    expect(buttons[1].textContent).toBe("Connect");
    expect(buttons[2].textContent).toBe("Connect");
    expect(buttons[2].disabled).toBe(true);
    await act(async () => {
      buttons[1].dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(connect).toHaveBeenCalledWith("cursor");
    expect(host.textContent).toContain("Written to /home/u/.cursor/mcp.json — restart your agent.");
  });

  it("disconnects a connected client and surfaces backend errors", async () => {
    mockApi();
    const disconnect = vi.spyOn(api, "mcpDisconnect").mockResolvedValue({ ok: false, error: "config locked" });
    const host = await renderCard();
    const buttons = Array.from(host.querySelectorAll("button"));
    await act(async () => {
      buttons[0].dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(disconnect).toHaveBeenCalledWith("claude-code");
    expect(host.textContent).toContain("config locked");
  });
});
