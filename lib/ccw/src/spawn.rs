//! The canonical `claude` spawn recipe (`reports/ccw-v1-recon.md` Deliverables
//! 3 + 4). CCW v1 OWNS claude invocation — it always spawns with the streaming
//! flag recipe, and this module is the single place that recipe is constructed.
//!
//! Two knobs the operator called out (INTENT #208):
//! - **stream-json always** — the daemon parses the live stream into CcwEvents.
//! - **tool clean-slate** — `--tools ""` zeroes every built-in, then a provided
//!   tool set is injected via `--agents` / plugin dirs and fenced with
//!   allow/deny lists. "Our entire Leverage OS needs to engineer its own tools."

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The claude binary CCW invokes. Honors `CCW_CLAUDE_BIN` (tests point it at a
/// fake stream-json emitter); defaults to `claude` on PATH.
pub fn claude_bin() -> String {
    std::env::var("CCW_CLAUDE_BIN")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claude".to_string())
}

/// The per-session tool policy (INTENT #208 clean-slate option).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolConfig {
    /// When true, start with ALL built-in tools deactivated (`--tools ""`).
    /// Leverage hands agents engineered tools only.
    #[serde(default)]
    pub clean_slate: bool,
    /// Explicit built-in tool set (e.g. `"Bash,Edit,Read"`). Ignored when
    /// `clean_slate` is true. `None` = claude's default (all built-ins).
    #[serde(default)]
    pub tools: Option<String>,
    /// Injected custom agents as a JSON object (`--agents <json>`), the no-MCP
    /// way to provide engineered capability.
    #[serde(default)]
    pub agents_json: Option<String>,
    /// Plugin directories carrying hooks/skills/commands/agents (`--plugin-dir`).
    #[serde(default)]
    pub plugin_dirs: Vec<String>,
    /// Allow-list fence (supports scoping like `Bash(git *)`).
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Deny-list fence.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
}

/// Everything CCW needs to construct one turn's `claude` invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnConfig {
    /// Model alias (`fable` | `opus` | `sonnet`) or full id.
    pub model: String,
    /// The CCW/claude session uuid (identity).
    pub session_id: String,
    /// Working directory the agent runs in.
    pub cwd: String,
    /// Role prompt appended via `--append-system-prompt`.
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    /// Extra `--add-dir` scoping (the cwd is always added implicitly by claude).
    #[serde(default)]
    pub add_dirs: Vec<String>,
    /// Permission mode (`plan` read-only, `bypassPermissions` unattended, …).
    /// `None` uses `--dangerously-skip-permissions` (aui's default).
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Tool clean-slate / injection policy.
    #[serde(default)]
    pub tools: ToolConfig,
    /// Optional native per-run spend ceiling (`--max-budget-usd`).
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    /// Optional fallback model for overload (`--fallback-model`).
    #[serde(default)]
    pub fallback_model: Option<String>,
}

impl SpawnConfig {
    /// Build the `claude` argument vector for a turn.
    ///
    /// `first_turn` selects `--session-id <uuid>` (mint) vs `--resume <uuid>`
    /// (continue). The prompt is NOT in argv — it is fed on stdin.
    pub fn build_args(&self, first_turn: bool) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();

        // Streaming print mode — always (the daemon parses the live stream).
        a.push("-p".into());
        a.push("--output-format".into());
        a.push("stream-json".into());
        a.push("--verbose".into());

        // Permission posture.
        match &self.permission_mode {
            Some(mode) => {
                a.push("--permission-mode".into());
                a.push(mode.clone());
            }
            None => a.push("--dangerously-skip-permissions".into()),
        }

        // Model.
        a.push("--model".into());
        a.push(self.model.clone());
        if let Some(fb) = &self.fallback_model {
            a.push("--fallback-model".into());
            a.push(fb.clone());
        }

        // Role prompt.
        if let Some(sp) = &self.append_system_prompt {
            a.push("--append-system-prompt".into());
            a.push(sp.clone());
        }

        // Session identity.
        if first_turn {
            a.push("--session-id".into());
        } else {
            a.push("--resume".into());
        }
        a.push(self.session_id.clone());

        // Directory scoping.
        for d in &self.add_dirs {
            a.push("--add-dir".into());
            a.push(d.clone());
        }

        // Native per-run spend cap.
        if let Some(b) = self.max_budget_usd {
            a.push("--max-budget-usd".into());
            a.push(format!("{b}"));
        }

        // ── Tool clean-slate / injection (recon Deliverable 4) ───────────
        if self.tools.clean_slate {
            // `--tools ""` disables every built-in. THE clean-slate switch.
            a.push("--tools".into());
            a.push(String::new());
        } else if let Some(t) = &self.tools.tools {
            a.push("--tools".into());
            a.push(t.clone());
        }
        if let Some(agents) = &self.tools.agents_json {
            a.push("--agents".into());
            a.push(agents.clone());
        }
        for pd in &self.tools.plugin_dirs {
            a.push("--plugin-dir".into());
            a.push(pd.clone());
        }
        for t in &self.tools.allowed_tools {
            a.push("--allowedTools".into());
            a.push(t.clone());
        }
        for t in &self.tools.disallowed_tools {
            a.push("--disallowedTools".into());
            a.push(t.clone());
        }

        a
    }

    /// The environment overrides for a spawn (recon Deliverable 3): strip the
    /// API key so subscription auth is used, disable auto-memory, and keep async
    /// subagents alive past claude's default 10-min ceilings (the idle-illusion
    /// fix). `config_dir` selects the account (None = machine default).
    pub fn env(&self, config_dir: Option<&str>) -> Vec<(String, Option<String>)> {
        let mut e: Vec<(String, Option<String>)> = vec![
            ("ANTHROPIC_API_KEY".into(), None), // remove
            ("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), Some("1".into())),
            ("CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS".into(), Some("0".into())),
            (
                "CLAUDE_ASYNC_AGENT_STALL_TIMEOUT_MS".into(),
                Some("3600000".into()),
            ),
        ];
        match config_dir {
            Some(d) => e.push(("CLAUDE_CONFIG_DIR".into(), Some(d.to_string()))),
            None => e.push(("CLAUDE_CONFIG_DIR".into(), None)),
        }
        e
    }
}

/// A registered per-session role/tool preset (kept as a map for the config file
/// to carry named toolsets; not wired to a rollup ref in v1 — see `rollup`).
pub type PresetMap = BTreeMap<String, ToolConfig>;
