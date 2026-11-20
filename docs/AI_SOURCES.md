# AI usage sources

Research for the AI usage collector (Milestone 6): where each AI coding tool
keeps its local logs, what is in them, and how to count usage from them without
double counting. This covers local files only; no tool is asked or queried.

**How this was checked (2026-11-18, macOS).** Installed tools: real files were
inspected with scripts that print only key names, JSON types, record-type
labels and counts, never values. No prompt, response, path or title text was
printed or stored. Tools not installed: official docs first, then well-known
third-party parsers where the docs say nothing. Those adapters were initially
marked **untested** here and in code.

Codex CLI was subsequently tested against real logs on Brian’s Mac.

Field names below are the on-disk names. "Normalised" means what Wrapped
stores: `input` = uncached input, `output` (incl. reasoning / thinking),
`cache_read`, `cache_write`.

## Summary

| Tool | On this Mac | Token counts on disk | Adapter |
|---|---|---|---|
| Claude Code | **yes**: 369 transcripts, 487 MB, 2026-10-28 → today | yes, per API response | tested |
| Google Antigravity (CLI `agy`) | **yes**: 13 conversations, 2026-11-02 → 11-12 | yes, per step (no cache write) | tested |
| GitHub Copilot CLI | folder only: `~/.copilot` has `config.json` and 8 process logs, no sessions | uncertain (see below) | untested |
| Gemini CLI | no (`~/.gemini` exists but holds only Antigravity) | yes, per message | untested |
| Codex CLI | **yes** | yes, cumulative per session | tested |
| Cursor CLI (`cursor-agent`) | no | **no** (sessions, time and model only) | untested |
| opencode | no | yes, per message | untested |

Env vars checked on this Mac: none of `CLAUDE_CONFIG_DIR`, `CODEX_HOME`,
`COPILOT_HOME`, `GEMINI_CLI_HOME`, `XDG_DATA_HOME`, `OPENCODE_DATA_DIR` is set.

## Claude Code (tested)

- **Location.** `~/.claude/projects/<encoded cwd>/<session uuid>.jsonl`, and
  subagent transcripts at `…/<session uuid>/subagents/agent-*.jsonl` (84 here).
  Windows: `%USERPROFILE%\.claude\projects\…`. `CLAUDE_CONFIG_DIR` replaces
  `~/.claude` entirely (official docs). The folder name encodes the cwd lossily
  (`/` and `.` become `-`), so the project comes from the record's `cwd`, not
  the folder name.
- **Format.** JSONL, append-only. Many record types (`user`, `assistant`,
  `system`, `attachment`, `api-request`, `cost-state`, `ai-title`,
  `continued-in`, `fork-context-ref`, …). Only `type == "assistant"` records
  with `message.usage` are counted.
- **Fields.**
  - timestamp: `timestamp` (ISO 8601, UTC `Z`)
  - model: `message.model` (`<synthetic>` = locally generated, zero tokens: skipped)
  - input: `message.usage.input_tokens` (already excludes cache)
  - output: `message.usage.output_tokens` (thinking is included; also broken out in `output_tokens_details.thinking_tokens`)
  - cache read: `message.usage.cache_read_input_tokens`
  - cache write: `message.usage.cache_creation_input_tokens` (split into `cache_creation.ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens`)
  - session: `sessionId`; project: `cwd` (last component kept)
  - ids: `message.id`, `requestId`, `uuid`
- **Ignored.** `cost-state` (a running per-session snapshot of `modelUsage`
  and `totalCostUSD`; counting it would double count, and Wrapped stores no
  cost); `message.usage.iterations[]` (a per-iteration breakdown of the same
  totals); `api-request` (request parameters, no usage).
- **Retention.** `cleanupPeriodDays` in `settings.json`, default 30 days,
  enforced at startup (the oldest file here is 21 days old). Community reports
  say subagent transcripts are not always cleaned up. This is the main reason
  to import promptly: after 30 days the numbers exist only in Wrapped.
- **Dedupe (measured).** One API response is written as several lines (one per
  content block) with the same `message.id` and identical usage: 24,664 usage
  lines, 11,471 unique `message.id`s, 13,193 repeats, **0** with different
  usage. 392 message ids appear in more than one file (resumed / continued /
  forked sessions and subagents copy earlier lines). So the key is
  `message.id` (+ `requestId` when present), deduped globally across files,
  not per file. Counting lines naively would more than double the totals.

## Google Antigravity CLI `agy` (tested)

- **Location.** `~/.gemini/antigravity-cli/`. Per conversation (uuid):
  `brain/<uuid>/.system_generated/logs/transcript_full.jsonl` (also
  `transcript.jsonl` and `chunks/…/<n>.jsonl`, which are copies / slices of
  the same steps and must not be read too), and `conversations/<uuid>.db`
  (SQLite, protobuf blobs). `history.jsonl` and `cache/last_conversations.json`
  map conversations to workspaces. Windows: `%USERPROFILE%\.gemini\antigravity-cli`
  (by analogy, not documented). No env var to move it (a feature request for a
  configurable `brain/` path exists). The Antigravity desktop app keeps a
  similar tree under `~/.gemini/antigravity/` (not installed here; would be
  untested).
- **Format.** JSONL, one record per step, written in `step_index` order.
- **Fields** (`transcript_full.jsonl`):
  - timestamp: `created_at` (ISO 8601 `Z`)
  - tokens: `input_tokens`, `output_tokens`, `cache_read_tokens`, present on
    `type == "PLANNER_RESPONSE"` (and a few `CHECKPOINT`) steps. Per step, not
    cumulative (values go down as often as up). `input_tokens` excludes cache
    (cache read is larger than input in 650 of 694 steps).
  - cache write: **not recorded**
  - model: **not in the transcript.** It is in `conversations/<uuid>.db`
    (SQLite, opened read-only and immutable): table `gen_metadata` (`idx`,
    `data` protobuf blob, `size`), one row per model call. In each blob, field
    1.19 is the model id (as written, e.g. `gemini-4-argon-trial`,
    `gemini-3.8-flash-n`, `claude-opus-5-5-medium`) and 1.4.2 / 1.4.3 / 1.4.5
    are that call's input / output / cache-read tokens. The blob also holds
    prompt and response text, which is never read (a small protobuf walker
    reads only those four fields). `idx` is the row order, **not** the
    transcript `step_index`, and row counts differ from the token-step counts
    by 0 to 1, so rows are joined to steps by their three token counts: of 637
    steps here, 634 match a row and none match two rows with different models.
    **Per step:** a matched step takes its row's model. A step with no match,
    or whose counts match rows of different models, takes the conversation's
    most used model (ties: first alphabetically); `unknown` when the database
    is missing or has no model id. A conversation can mix models (one here
    used Claude Opus and two Gemini models), so the split is per step.
    `settings.json` has `model` (the current default only). Events counted
    before this was added keep `unknown`.
  - session: the `brain/<uuid>` folder name (= `conversations/<uuid>.db`).
  - project: `history.jsonl` has `conversationId` + `workspace` (a path) per
    prompt. That record also holds the prompt text in `display`, which the
    adapter must not read beyond parsing the line.
  - `type`, `source`, `status` are enums (`PLANNER_RESPONSE`, `MODEL`, `DONE`,
    `RUNNING`, …). A step can be written as `RUNNING` without tokens; only
    steps with token fields count.
- **Retention.** Not documented; nothing older than 16 days here, but that is
  when it was installed.
- **Dedupe.** `(conversation uuid, step_index)`. Only `transcript_full.jsonl`
  is read (the chunk files repeat the same steps).

## GitHub Copilot CLI (untested)

- **Location.** `~/.copilot/` (`$HOME/.copilot`; Windows
  `%USERPROFILE%\.copilot`). `COPILOT_HOME` replaces it (official docs).
  Sessions: `session-state/<session id>/events.jsonl` (versions before 1.0:
  `session-state/<id>.jsonl`). Also `session-store.db` (SQLite index) and
  `logs/process-<ts>-<pid>.log`.
- **On this Mac.** `config.json`, `logs/` with 8 small process logs
  (2026-10-07 → 11-13), no `session-state/`, and no `copilot` binary on PATH.
  The logs only say a runtime started and stopped (no token or usage lines),
  so something started the Copilot runtime but no CLI session was saved.
- **Format.** JSONL event log; events have a `type` (`session.start`,
  `user.message`, `assistant.message`, …) and a timestamp.
- **Tokens.** The SDK docs call `assistant.usage` the per-call token event but
  say it is ephemeral (not replayed on resume), so it may not be on disk. A
  bug report mentions `session.usage_checkpoint` events in `events.jsonl` and
  an `assistant_usage_events` table in `session-store.db`; neither is
  documented. Plan: count per-call `assistant.usage` if present, else
  per-session `modelMetrics`-style checkpoint totals (cumulative, so deltas);
  else sessions only.
- **Retention.** No automatic pruning documented.
- **Dedupe.** Event ids if present; cumulative checkpoints by
  `(session, model, cumulative total)`.

## Gemini CLI (untested)

- **Location.** `~/.gemini/tmp/<project>/chats/`. Older builds:
  `session-<date>-<id>.json` (one JSON document per session); newer builds
  are reported to write append-only `.jsonl` with the old files still
  readable. `<project>` is a SHA-256 of the project path in older builds;
  newer builds are reported to use a readable slug with a `.project_root`
  file. Windows: `%USERPROFILE%\.gemini\tmp\…`. `GEMINI_CLI_HOME` is reported
  to move the home folder; not confirmed in the official docs.
- **Fields** (JSON): top level `sessionId`, `projectHash`, `startTime`,
  `lastUpdated`, `messages[]`; message `id`, `timestamp`, `type`
  (`user` / `gemini` / `info`), and on `gemini` messages `model` and
  `tokens { input, output, cached, thoughts, tool, total }`.
  Normalised: input = `input − cached` (Gemini's prompt count includes
  cached tokens), output = `output + thoughts`, cache read = `cached`, cache
  write = none. Project: `.project_root` if present, else unknown (a hash
  cannot be turned back into a name).
- **Retention.** `general.sessionRetention` in `settings.json`; the docs show
  `maxAge` default `"30d"`.
- **Dedupe.** `(sessionId, message id)`. The JSON format rewrites the whole
  file, so it is re-read whole when its size or mtime changes, relying on the
  id dedupe.

## Codex CLI (tested)

- **Location.** `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`,
  `CODEX_HOME` default `~/.codex` (Windows `%USERPROFILE%\.codex`). Some
  sources mention `.jsonl.zst` in some versions: those are skipped and
  reported as unsupported (no zstd dependency).
- **Format.** JSONL; each line `{ timestamp, type, payload }`. Types:
  `session_meta` (payload `id`, `cwd`, …), `turn_context` (`cwd`, `model`),
  `response_item`, `event_msg`.
- **Tokens.** `event_msg` with `payload.type == "token_count"`,
  `payload.info.total_token_usage { input_tokens, cached_input_tokens,
  output_tokens, reasoning_output_tokens, total_tokens }` and
  `last_token_usage` (same fields). Totals are **cumulative per session**;
  `info` can be null; the same total can be emitted twice. Normalised: input =
  `input_tokens − cached_input_tokens`, cache read = `cached_input_tokens`,
  output = `output_tokens` (reasoning already included), cache write = none.
  Model: latest `turn_context.payload.model`. Project: `session_meta.payload.cwd`.
- **Retention.** No automatic cleanup documented (issues report multi-GB logs).
- **Dedupe.** Delta of `total_token_usage` against the previous event in that
  session; skip events whose total did not change. Key
  `(session id, cumulative total_tokens)`, so a resumed / forked session that
  replays earlier history does not count it again.

## Cursor CLI `cursor-agent` (untested)

- **Location.** `~/.cursor/chats/<workspace hash>/<chat id>/store.db`
  (SQLite: `meta` table with hex-encoded JSON, `blobs` table with
  content-addressed message blobs). Windows `%USERPROFILE%\.cursor\chats`.
  No documented env var.
- **Tokens.** **Not on disk**, according to Cursor's own forum (token usage is
  only in `--print --output-format json` output; a feature request asks for it
  in local transcripts). `meta` reportedly has `lastUsedModel` and a
  creation time.
- **Recorded instead.** Sessions, first / last time (creation time and the
  db file's mtime), model. The card says "no token counts" for this tool.
  Project: unknown (the folder is a hash of the workspace).
- **Dedupe.** Chat id.
- **Retention.** Not documented.

## opencode (untested)

- **Location.** `$XDG_DATA_HOME/opencode` (default `~/.local/share/opencode`,
  same on Windows under `%USERPROFILE%`). `OPENCODE_DATA_DIR` is reported to
  override it; `opencode debug paths` prints the real one. File storage:
  `storage/message/<sessionID>/<messageID>.json` (also `storage/session/…`,
  `storage/part/…`). Some versions use a SQLite database in the same folder
  instead (message JSON in a `data` column); both shapes are supported.
- **Fields** (assistant message): `id`, `sessionID`, `role`, `modelID`,
  `providerID`, `time.created` / `time.completed` (Unix ms),
  `tokens { input, output, reasoning, cache { read, write } }`,
  `path { cwd, root }`. Normalised: output = `output + reasoning`.
  `step-finish` parts repeat per-step tokens: not read (would double count).
- **Retention.** None (storage grows without bound per an open issue).
- **Dedupe.** Message id. Message files are rewritten while streaming, so a
  message is counted once it has `time.completed`.

## Shared rules for the adapters

- **Read only.** Never write, lock or rename anything in a tool's folder.
  SQLite sources are opened read-only and immutable-safe (no WAL writes into a
  running tool's database).
- **Never keep text.** Lines are parsed and only the fields above are taken.
  Prompt / response / tool / title fields are never copied out of the parser.
  The project is reduced to the last path component before it leaves the
  adapter.
- **Incremental.** JSONL: resume at the stored byte offset of the last
  complete line, if the file's head (hash of its first line) is unchanged and
  it did not shrink; otherwise read it again from the start and let the id
  dedupe drop what was counted before. Whole-file formats (Gemini JSON,
  opencode message files, SQLite): re-read when size or mtime changes.
- **Imported numbers stay.** A deleted or rotated log never removes what was
  imported.

## Sources

- Claude Code `.claude` directory: https://code.claude.com/docs/en/claude-directory
- Claude Code 30-day cleanup reports: https://github.com/anthropics/claude-code/issues/62272, https://brycewatson.com/blog/28-claude-code-deletes-old-logs
- Codex rollout format: https://lin-guanguo.github.io/llm-memory-research/agent-cli/codex-session-files/, https://codex.danielvaughan.com/2026/03/30/codex-cli-session-analytics-jsonl-rollout, ccusage Codex guide https://baa0580d-ccusage-guide.ryoppippi.workers.dev/guide/codex/
- Gemini CLI sessions: https://geminicli.com/docs/cli/session-management/, https://lin-guanguo.github.io/llm-memory-research/agent-cli/gemini-session-file-schema/, https://www.mintlify.com/google-gemini/gemini-cli/cli/sessions
- Antigravity CLI: https://www.antigravity.google/docs/cli/conversations, https://antigravity.google/docs/settings?app=cli, https://dev.to/iromin/antigravity-cli-tutorial-series-part-2-conversations-conversations-and-conversations-j39
- Copilot CLI config dir: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference, SDK usage events: https://docs.github.com/en/copilot/how-tos/copilot-sdk/features/usage-and-billing, ledger report: https://github.com/github/copilot-cli/issues/4351
- Cursor CLI: https://forum.cursor.com/t/record-token-usage-in-local-agent-transcripts/175149, https://forum.cursor.com/t/include-token-usage-in-stream-json-output/146980, https://hn.jakechampion.name/reader/https://vibe-replay.com/blog/cursor-local-storage
- opencode storage: https://forums.basehub.com/anomalyco/opencode/24, https://www.mintlify.com/ryoppippi/ccusage/family/opencode, https://github.com/anomalyco/opencode/issues/22110
