// Display names for AI usage keys (shared by the Overview cards and recaps).

export const AI_TOOLS: Record<string, string> = {
  claude: "Claude Code",
  codex: "Codex",
  copilot: "Copilot CLI",
  antigravity: "Antigravity",
  gemini: "Gemini CLI",
  cursor: "Cursor CLI",
  opencode: "opencode",
};

export const aiToolName = (id: string): string => AI_TOOLS[id] ?? id;

// "unknown" is stored when an Antigravity conversation has no model id in its database.
export const modelLabel = (k: string): string => (k === "unknown" ? "Antigravity (model unknown)" : k);
