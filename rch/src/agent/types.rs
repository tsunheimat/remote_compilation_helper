//! Agent type definitions.
//!
//! Defines the core types for AI coding agent detection and configuration.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Supported AI coding agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    /// Claude Code (Anthropic's official CLI)
    ClaudeCode,
    /// Gemini CLI (Google's CLI)
    GeminiCli,
    /// Codex CLI (OpenAI)
    CodexCli,
    /// Hermes terminal tool integration.
    Hermes,
    /// Cursor (AI-powered IDE)
    Cursor,
    /// Continue.dev (VS Code extension)
    ContinueDev,
    /// Windsurf (Codeium)
    Windsurf,
    /// Aider (terminal-based assistant)
    Aider,
    /// Cline (VS Code extension)
    Cline,
}

impl AgentKind {
    /// All known agent kinds.
    pub const ALL: &'static [AgentKind] = &[
        AgentKind::ClaudeCode,
        AgentKind::GeminiCli,
        AgentKind::CodexCli,
        AgentKind::Hermes,
        AgentKind::Cursor,
        AgentKind::ContinueDev,
        AgentKind::Windsurf,
        AgentKind::Aider,
        AgentKind::Cline,
    ];

    /// Human-readable name.
    pub fn name(&self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::GeminiCli => "Gemini CLI",
            AgentKind::CodexCli => "Codex CLI",
            AgentKind::Hermes => "Hermes",
            AgentKind::Cursor => "Cursor",
            AgentKind::ContinueDev => "Continue.dev",
            AgentKind::Windsurf => "Windsurf",
            AgentKind::Aider => "Aider",
            AgentKind::Cline => "Cline",
        }
    }

    /// CLI identifier (kebab-case).
    pub fn id(&self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude-code",
            AgentKind::GeminiCli => "gemini-cli",
            AgentKind::CodexCli => "codex-cli",
            AgentKind::Hermes => "hermes",
            AgentKind::Cursor => "cursor",
            AgentKind::ContinueDev => "continue-dev",
            AgentKind::Windsurf => "windsurf",
            AgentKind::Aider => "aider",
            AgentKind::Cline => "cline",
        }
    }

    /// Parse from CLI identifier.
    pub fn from_id(id: &str) -> Option<AgentKind> {
        match id.to_lowercase().as_str() {
            "claude-code" | "claude" => Some(AgentKind::ClaudeCode),
            "gemini-cli" | "gemini" => Some(AgentKind::GeminiCli),
            "codex-cli" | "codex" => Some(AgentKind::CodexCli),
            "hermes" => Some(AgentKind::Hermes),
            "cursor" => Some(AgentKind::Cursor),
            "continue-dev" | "continue" => Some(AgentKind::ContinueDev),
            "windsurf" => Some(AgentKind::Windsurf),
            "aider" => Some(AgentKind::Aider),
            "cline" => Some(AgentKind::Cline),
            _ => None,
        }
    }

    /// Level of hook support for this agent.
    pub fn hook_support(&self) -> HookSupport {
        match self {
            AgentKind::ClaudeCode => HookSupport::Full,
            AgentKind::GeminiCli => HookSupport::Full,
            AgentKind::CodexCli => HookSupport::Full,
            AgentKind::Hermes => HookSupport::Full,
            AgentKind::ContinueDev => HookSupport::Partial,
            AgentKind::Cursor => HookSupport::DetectionOnly,
            AgentKind::Windsurf => HookSupport::DetectionOnly,
            AgentKind::Aider => HookSupport::DetectionOnly,
            AgentKind::Cline => HookSupport::DetectionOnly,
        }
    }

    /// Default configuration path for this agent.
    pub fn default_config_path(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        match self {
            AgentKind::ClaudeCode => Some(home.join(".claude").join("settings.json")),
            AgentKind::GeminiCli => Some(home.join(".gemini").join("settings.json")),
            AgentKind::CodexCli => Some(home.join(".codex").join("hooks.json")),
            AgentKind::Hermes => Some(home.join(".hermes").join("plugins").join("rch-rewrite")),
            AgentKind::ContinueDev => Some(home.join(".continue").join("config.json")),
            AgentKind::Cursor => Some(home.join(".cursor")),
            AgentKind::Windsurf => Some(home.join(".windsurf")),
            AgentKind::Aider => Some(home.join(".aider.conf.yml")),
            AgentKind::Cline => None, // VS Code extension, managed differently
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Level of hook support for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookSupport {
    /// Full PreToolUse hook support with JSON protocol.
    Full,
    /// Partial support (some hook functionality).
    Partial,
    /// Can detect but cannot install hooks.
    DetectionOnly,
    /// No hook support at all.
    None,
}

impl HookSupport {
    /// Human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            HookSupport::Full => "Full hook support (PreToolUse)",
            HookSupport::Partial => "Partial hook support",
            HookSupport::DetectionOnly => "Detection only",
            HookSupport::None => "Not supported",
        }
    }

    /// Whether hooks can be installed for this support level.
    pub fn can_install_hook(&self) -> bool {
        matches!(self, HookSupport::Full | HookSupport::Partial)
    }
}

impl fmt::Display for HookSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Information about a detected AI coding agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedAgent {
    /// The type of agent.
    pub kind: AgentKind,
    /// Version string if detected.
    pub version: Option<String>,
    /// Path to the agent's configuration file/directory.
    pub config_path: Option<PathBuf>,
    /// Level of hook support.
    pub hook_support: HookSupport,
    /// How the agent was detected.
    pub detection_method: DetectionMethod,
}

impl DetectedAgent {
    /// Create a new detected agent.
    pub fn new(kind: AgentKind) -> Self {
        Self {
            kind,
            version: None,
            config_path: kind.default_config_path(),
            hook_support: kind.hook_support(),
            detection_method: DetectionMethod::NotDetected,
        }
    }

    /// Set the version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the config path.
    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Set the detection method.
    pub fn with_detection_method(mut self, method: DetectionMethod) -> Self {
        self.detection_method = method;
        self
    }
}

/// How an agent was detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionMethod {
    /// Found via PATH lookup (which command).
    PathLookup,
    /// Found via config file existence.
    ConfigFile,
    /// Found via process detection.
    ProcessDetection,
    /// Not actually detected (placeholder).
    NotDetected,
}

impl fmt::Display for DetectionMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetectionMethod::PathLookup => write!(f, "PATH lookup"),
            DetectionMethod::ConfigFile => write!(f, "config file"),
            DetectionMethod::ProcessDetection => write!(f, "process detection"),
            DetectionMethod::NotDetected => write!(f, "not detected"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== TEST: AgentKind Constants =====

    #[test]
    fn test_agent_kind_all() {
        eprintln!("TEST START: test_agent_kind_all");
        assert_eq!(AgentKind::ALL.len(), 9);
        assert_eq!(AgentKind::from_id("hermes"), Some(AgentKind::Hermes));
        assert_eq!(AgentKind::Hermes.hook_support(), HookSupport::Full);
        eprintln!("TEST PASS: test_agent_kind_all - verified 9 agent kinds");
    }

    #[test]
    fn test_agent_kind_all_unique() {
        eprintln!("TEST START: test_agent_kind_all_unique");
        let mut seen = std::collections::HashSet::new();
        for kind in AgentKind::ALL {
            assert!(seen.insert(kind), "Duplicate agent kind in ALL: {:?}", kind);
        }
        eprintln!("TEST PASS: test_agent_kind_all_unique - all 8 kinds are unique");
    }

    // ===== TEST: AgentKind from_id =====

    #[test]
    fn test_agent_kind_from_id() {
        eprintln!("TEST START: test_agent_kind_from_id");
        assert_eq!(
            AgentKind::from_id("claude-code"),
            Some(AgentKind::ClaudeCode)
        );
        assert_eq!(AgentKind::from_id("claude"), Some(AgentKind::ClaudeCode));
        assert_eq!(
            AgentKind::from_id("CLAUDE-CODE"),
            Some(AgentKind::ClaudeCode)
        );
        assert_eq!(AgentKind::from_id("unknown"), None);
        eprintln!("TEST PASS: test_agent_kind_from_id");
    }

    #[test]
    fn test_agent_kind_from_id_all_variants() {
        eprintln!("TEST START: test_agent_kind_from_id_all_variants");
        // Test all aliases for each agent
        let test_cases = [
            ("claude-code", AgentKind::ClaudeCode),
            ("claude", AgentKind::ClaudeCode),
            ("gemini-cli", AgentKind::GeminiCli),
            ("gemini", AgentKind::GeminiCli),
            ("codex-cli", AgentKind::CodexCli),
            ("codex", AgentKind::CodexCli),
            ("cursor", AgentKind::Cursor),
            ("continue-dev", AgentKind::ContinueDev),
            ("continue", AgentKind::ContinueDev),
            ("windsurf", AgentKind::Windsurf),
            ("aider", AgentKind::Aider),
            ("cline", AgentKind::Cline),
        ];

        for (input, expected) in test_cases {
            assert_eq!(
                AgentKind::from_id(input),
                Some(expected),
                "Failed for input: {}",
                input
            );
            eprintln!("  Verified: {} -> {:?}", input, expected);
        }
        eprintln!("TEST PASS: test_agent_kind_from_id_all_variants");
    }

    #[test]
    fn test_agent_kind_from_id_case_insensitive() {
        eprintln!("TEST START: test_agent_kind_from_id_case_insensitive");
        assert_eq!(AgentKind::from_id("CLAUDE"), Some(AgentKind::ClaudeCode));
        assert_eq!(AgentKind::from_id("Claude"), Some(AgentKind::ClaudeCode));
        assert_eq!(AgentKind::from_id("GEMINI-CLI"), Some(AgentKind::GeminiCli));
        assert_eq!(AgentKind::from_id("Codex-Cli"), Some(AgentKind::CodexCli));
        eprintln!("TEST PASS: test_agent_kind_from_id_case_insensitive");
    }

    #[test]
    fn test_agent_kind_from_id_invalid() {
        eprintln!("TEST START: test_agent_kind_from_id_invalid");
        let invalid = ["", "unknown", "gpt-4", "copilot", "  claude  ", "claude-"];
        for input in invalid {
            assert_eq!(
                AgentKind::from_id(input),
                None,
                "Should return None for: {:?}",
                input
            );
        }
        eprintln!("TEST PASS: test_agent_kind_from_id_invalid");
    }

    #[test]
    fn test_agent_kind_roundtrip() {
        eprintln!("TEST START: test_agent_kind_roundtrip");
        for kind in AgentKind::ALL {
            let id = kind.id();
            assert_eq!(AgentKind::from_id(id), Some(*kind));
            eprintln!("  Roundtrip: {:?} -> {} -> {:?}", kind, id, kind);
        }
        eprintln!("TEST PASS: test_agent_kind_roundtrip");
    }

    // ===== TEST: AgentKind name/id/display =====

    #[test]
    fn test_agent_kind_name_not_empty() {
        eprintln!("TEST START: test_agent_kind_name_not_empty");
        for kind in AgentKind::ALL {
            assert!(!kind.name().is_empty(), "Agent {:?} has empty name", kind);
            eprintln!("  {:?}.name() = {:?}", kind, kind.name());
        }
        eprintln!("TEST PASS: test_agent_kind_name_not_empty");
    }

    #[test]
    fn test_agent_kind_id_is_kebab_case() {
        eprintln!("TEST START: test_agent_kind_id_is_kebab_case");
        for kind in AgentKind::ALL {
            let id = kind.id();
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "Agent {:?} id {:?} is not kebab-case",
                kind,
                id
            );
        }
        eprintln!("TEST PASS: test_agent_kind_id_is_kebab_case");
    }

    #[test]
    fn test_agent_kind_display_matches_name() {
        eprintln!("TEST START: test_agent_kind_display_matches_name");
        for kind in AgentKind::ALL {
            assert_eq!(
                format!("{}", kind),
                kind.name(),
                "Display doesn't match name for {:?}",
                kind
            );
        }
        eprintln!("TEST PASS: test_agent_kind_display_matches_name");
    }

    // ===== TEST: HookSupport =====

    #[test]
    fn test_hook_support_can_install() {
        eprintln!("TEST START: test_hook_support_can_install");
        assert!(HookSupport::Full.can_install_hook());
        assert!(HookSupport::Partial.can_install_hook());
        assert!(!HookSupport::DetectionOnly.can_install_hook());
        assert!(!HookSupport::None.can_install_hook());
        eprintln!("TEST PASS: test_hook_support_can_install");
    }

    #[test]
    fn test_hook_support_description_not_empty() {
        eprintln!("TEST START: test_hook_support_description_not_empty");
        let variants = [
            HookSupport::Full,
            HookSupport::Partial,
            HookSupport::DetectionOnly,
            HookSupport::None,
        ];
        for support in variants {
            assert!(
                !support.description().is_empty(),
                "{:?} has empty description",
                support
            );
            eprintln!(
                "  {:?}.description() = {:?}",
                support,
                support.description()
            );
        }
        eprintln!("TEST PASS: test_hook_support_description_not_empty");
    }

    #[test]
    fn test_hook_support_display() {
        eprintln!("TEST START: test_hook_support_display");
        assert_eq!(
            format!("{}", HookSupport::Full),
            "Full hook support (PreToolUse)"
        );
        assert_eq!(format!("{}", HookSupport::Partial), "Partial hook support");
        assert_eq!(format!("{}", HookSupport::DetectionOnly), "Detection only");
        assert_eq!(format!("{}", HookSupport::None), "Not supported");
        eprintln!("TEST PASS: test_hook_support_display");
    }

    // ===== TEST: AgentKind -> HookSupport mapping =====

    #[test]
    fn test_agent_hook_support_mapping() {
        eprintln!("TEST START: test_agent_hook_support_mapping");
        // Full support agents
        assert_eq!(AgentKind::ClaudeCode.hook_support(), HookSupport::Full);
        assert_eq!(AgentKind::GeminiCli.hook_support(), HookSupport::Full);
        assert_eq!(AgentKind::CodexCli.hook_support(), HookSupport::Full);

        // Partial support
        assert_eq!(AgentKind::ContinueDev.hook_support(), HookSupport::Partial);

        // Detection only
        assert_eq!(AgentKind::Cursor.hook_support(), HookSupport::DetectionOnly);
        assert_eq!(
            AgentKind::Windsurf.hook_support(),
            HookSupport::DetectionOnly
        );
        assert_eq!(AgentKind::Aider.hook_support(), HookSupport::DetectionOnly);
        assert_eq!(AgentKind::Cline.hook_support(), HookSupport::DetectionOnly);

        eprintln!("TEST PASS: test_agent_hook_support_mapping");
    }

    // ===== TEST: DetectedAgent builder =====

    #[test]
    fn test_detected_agent_builder() {
        eprintln!("TEST START: test_detected_agent_builder");
        let agent = DetectedAgent::new(AgentKind::ClaudeCode)
            .with_version("1.0.0")
            .with_detection_method(DetectionMethod::PathLookup);

        assert_eq!(agent.kind, AgentKind::ClaudeCode);
        assert_eq!(agent.version, Some("1.0.0".to_string()));
        assert_eq!(agent.detection_method, DetectionMethod::PathLookup);
        eprintln!("TEST PASS: test_detected_agent_builder");
    }

    #[test]
    fn test_detected_agent_new_defaults() {
        eprintln!("TEST START: test_detected_agent_new_defaults");
        let agent = DetectedAgent::new(AgentKind::CodexCli);

        assert_eq!(agent.kind, AgentKind::CodexCli);
        assert_eq!(agent.version, None);
        assert_eq!(agent.detection_method, DetectionMethod::NotDetected);
        assert_eq!(agent.hook_support, HookSupport::Full);
        // Config path should be populated from default
        assert!(agent.config_path.is_some());
        eprintln!("  Default config_path: {:?}", agent.config_path);
        eprintln!("TEST PASS: test_detected_agent_new_defaults");
    }

    #[test]
    fn test_detected_agent_with_config_path() {
        eprintln!("TEST START: test_detected_agent_with_config_path");
        let custom_path = PathBuf::from("/custom/config/path");
        let agent = DetectedAgent::new(AgentKind::Aider).with_config_path(custom_path.clone());

        assert_eq!(agent.config_path, Some(custom_path));
        eprintln!("TEST PASS: test_detected_agent_with_config_path");
    }

    #[test]
    fn test_detected_agent_all_agents_have_hook_support() {
        eprintln!("TEST START: test_detected_agent_all_agents_have_hook_support");
        for kind in AgentKind::ALL {
            let agent = DetectedAgent::new(*kind);
            // hook_support should match the kind's hook_support
            assert_eq!(
                agent.hook_support,
                kind.hook_support(),
                "Mismatch for {:?}",
                kind
            );
            eprintln!("  {:?} -> {:?}", kind, agent.hook_support);
        }
        eprintln!("TEST PASS: test_detected_agent_all_agents_have_hook_support");
    }

    // ===== TEST: DetectionMethod =====

    #[test]
    fn test_detection_method_display() {
        eprintln!("TEST START: test_detection_method_display");
        assert_eq!(format!("{}", DetectionMethod::PathLookup), "PATH lookup");
        assert_eq!(format!("{}", DetectionMethod::ConfigFile), "config file");
        assert_eq!(
            format!("{}", DetectionMethod::ProcessDetection),
            "process detection"
        );
        assert_eq!(format!("{}", DetectionMethod::NotDetected), "not detected");
        eprintln!("TEST PASS: test_detection_method_display");
    }

    // ===== TEST: Serialization/Deserialization =====

    #[test]
    fn test_agent_kind_serde_roundtrip() {
        eprintln!("TEST START: test_agent_kind_serde_roundtrip");
        for kind in AgentKind::ALL {
            let json = serde_json::to_string(kind).expect("serialize");
            let parsed: AgentKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(*kind, parsed, "Roundtrip failed for {:?}", kind);
            eprintln!("  {:?} <-> {}", kind, json);
        }
        eprintln!("TEST PASS: test_agent_kind_serde_roundtrip");
    }

    #[test]
    fn test_agent_kind_serde_kebab_case() {
        eprintln!("TEST START: test_agent_kind_serde_kebab_case");
        // AgentKind uses kebab-case for serde
        let json = serde_json::to_string(&AgentKind::ClaudeCode).unwrap();
        assert_eq!(json, "\"claude-code\"");

        let json = serde_json::to_string(&AgentKind::GeminiCli).unwrap();
        assert_eq!(json, "\"gemini-cli\"");

        let json = serde_json::to_string(&AgentKind::ContinueDev).unwrap();
        assert_eq!(json, "\"continue-dev\"");

        eprintln!("TEST PASS: test_agent_kind_serde_kebab_case");
    }

    #[test]
    fn test_hook_support_serde_roundtrip() {
        eprintln!("TEST START: test_hook_support_serde_roundtrip");
        let variants = [
            HookSupport::Full,
            HookSupport::Partial,
            HookSupport::DetectionOnly,
            HookSupport::None,
        ];
        for support in variants {
            let json = serde_json::to_string(&support).expect("serialize");
            let parsed: HookSupport = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(support, parsed, "Roundtrip failed for {:?}", support);
            eprintln!("  {:?} <-> {}", support, json);
        }
        eprintln!("TEST PASS: test_hook_support_serde_roundtrip");
    }

    #[test]
    fn test_hook_support_serde_snake_case() {
        eprintln!("TEST START: test_hook_support_serde_snake_case");
        // HookSupport uses snake_case for serde
        assert_eq!(
            serde_json::to_string(&HookSupport::Full).unwrap(),
            "\"full\""
        );
        assert_eq!(
            serde_json::to_string(&HookSupport::Partial).unwrap(),
            "\"partial\""
        );
        assert_eq!(
            serde_json::to_string(&HookSupport::DetectionOnly).unwrap(),
            "\"detection_only\""
        );
        assert_eq!(
            serde_json::to_string(&HookSupport::None).unwrap(),
            "\"none\""
        );
        eprintln!("TEST PASS: test_hook_support_serde_snake_case");
    }

    #[test]
    fn test_detection_method_serde_roundtrip() {
        eprintln!("TEST START: test_detection_method_serde_roundtrip");
        let methods = [
            DetectionMethod::PathLookup,
            DetectionMethod::ConfigFile,
            DetectionMethod::ProcessDetection,
            DetectionMethod::NotDetected,
        ];
        for method in methods {
            let json = serde_json::to_string(&method).expect("serialize");
            let parsed: DetectionMethod = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(method, parsed, "Roundtrip failed for {:?}", method);
            eprintln!("  {:?} <-> {}", method, json);
        }
        eprintln!("TEST PASS: test_detection_method_serde_roundtrip");
    }

    #[test]
    fn test_detection_method_serde_snake_case() {
        eprintln!("TEST START: test_detection_method_serde_snake_case");
        assert_eq!(
            serde_json::to_string(&DetectionMethod::PathLookup).unwrap(),
            "\"path_lookup\""
        );
        assert_eq!(
            serde_json::to_string(&DetectionMethod::ConfigFile).unwrap(),
            "\"config_file\""
        );
        assert_eq!(
            serde_json::to_string(&DetectionMethod::ProcessDetection).unwrap(),
            "\"process_detection\""
        );
        assert_eq!(
            serde_json::to_string(&DetectionMethod::NotDetected).unwrap(),
            "\"not_detected\""
        );
        eprintln!("TEST PASS: test_detection_method_serde_snake_case");
    }

    #[test]
    fn test_detected_agent_serde_roundtrip() {
        eprintln!("TEST START: test_detected_agent_serde_roundtrip");
        let agent = DetectedAgent::new(AgentKind::ClaudeCode)
            .with_version("1.2.3")
            .with_config_path("/home/user/.claude/settings.json")
            .with_detection_method(DetectionMethod::PathLookup);

        let json = serde_json::to_string(&agent).expect("serialize");
        eprintln!("  Serialized: {}", json);

        let parsed: DetectedAgent = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(agent.kind, parsed.kind);
        assert_eq!(agent.version, parsed.version);
        assert_eq!(agent.config_path, parsed.config_path);
        assert_eq!(agent.hook_support, parsed.hook_support);
        assert_eq!(agent.detection_method, parsed.detection_method);

        eprintln!("TEST PASS: test_detected_agent_serde_roundtrip");
    }

    #[test]
    fn test_detected_agent_serde_minimal() {
        eprintln!("TEST START: test_detected_agent_serde_minimal");
        // Test with minimal fields (no version, no custom config path)
        let agent = DetectedAgent::new(AgentKind::Aider);

        let json = serde_json::to_string(&agent).expect("serialize");
        eprintln!("  Serialized: {}", json);

        let parsed: DetectedAgent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(agent.kind, parsed.kind);
        assert_eq!(agent.version, parsed.version); // Should be None
        assert_eq!(agent.detection_method, parsed.detection_method);

        eprintln!("TEST PASS: test_detected_agent_serde_minimal");
    }

    // ===== TEST: AgentKind default_config_path =====

    #[test]
    fn test_agent_kind_config_paths_consistency() {
        eprintln!("TEST START: test_agent_kind_config_paths_consistency");
        // All agents except Cline should have a default config path
        for kind in AgentKind::ALL {
            let path = kind.default_config_path();
            match kind {
                AgentKind::Cline => {
                    assert!(path.is_none(), "Cline should not have default config path");
                }
                _ => {
                    // Note: path may be None if HOME is not set, but if it's Some, validate it
                    if let Some(p) = &path {
                        eprintln!("  {:?} -> {:?}", kind, p);
                        // Path should contain the agent's config directory/file name
                        let path_str = p.to_string_lossy();
                        match kind {
                            AgentKind::ClaudeCode => assert!(path_str.contains(".claude")),
                            AgentKind::GeminiCli => assert!(path_str.contains(".gemini")),
                            AgentKind::CodexCli => assert!(path_str.ends_with(".codex/hooks.json")),
                            AgentKind::Hermes => {
                                assert!(path_str.ends_with(".hermes/plugins/rch-rewrite"))
                            }
                            AgentKind::ContinueDev => assert!(path_str.contains(".continue")),
                            AgentKind::Cursor => assert!(path_str.contains(".cursor")),
                            AgentKind::Windsurf => assert!(path_str.contains(".windsurf")),
                            AgentKind::Aider => assert!(path_str.contains(".aider")),
                            AgentKind::Cline => unreachable!(),
                        }
                    }
                }
            }
        }
        eprintln!("TEST PASS: test_agent_kind_config_paths_consistency");
    }
}
