//! Agent detection functions.
//!
//! Provides functions to detect installed AI coding agents on the system.

use super::types::{AgentKind, DetectedAgent, DetectionMethod};
use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Detect all installed AI coding agents.
///
/// Returns a list of detected agents with their version and configuration info.
pub fn detect_agents() -> Result<Vec<DetectedAgent>> {
    let mut agents = Vec::new();

    for kind in AgentKind::ALL {
        if let Some(agent) = detect_single_agent(*kind)? {
            agents.push(agent);
        }
    }

    Ok(agents)
}

/// Detect a specific agent by kind.
///
/// Returns `Some(DetectedAgent)` if the agent is installed, `None` otherwise.
pub fn detect_single_agent(kind: AgentKind) -> Result<Option<DetectedAgent>> {
    match kind {
        AgentKind::ClaudeCode => detect_claude_code(),
        AgentKind::GeminiCli => detect_gemini_cli(),
        AgentKind::CodexCli => detect_codex_cli(),
        AgentKind::Hermes => detect_hermes(),
        AgentKind::Cursor => detect_cursor(),
        AgentKind::ContinueDev => detect_continue_dev(),
        AgentKind::Windsurf => detect_windsurf(),
        AgentKind::Aider => detect_aider(),
        AgentKind::Cline => detect_cline(),
    }
}

/// Detect Claude Code installation.
fn detect_claude_code() -> Result<Option<DetectedAgent>> {
    // Try to find claude in PATH
    if let Some(version) = get_command_version("claude", &["--version"]) {
        let agent = DetectedAgent::new(AgentKind::ClaudeCode)
            .with_version(version)
            .with_detection_method(DetectionMethod::PathLookup);
        return Ok(Some(agent));
    }

    // Check for config directory
    if let Some(config_path) = AgentKind::ClaudeCode.default_config_path()
        && config_path.parent().map(|p| p.exists()).unwrap_or(false)
    {
        let agent = DetectedAgent::new(AgentKind::ClaudeCode)
            .with_detection_method(DetectionMethod::ConfigFile);
        return Ok(Some(agent));
    }

    Ok(None)
}

/// Detect Gemini CLI installation.
fn detect_gemini_cli() -> Result<Option<DetectedAgent>> {
    // Try to find gemini in PATH
    if let Some(version) = get_command_version("gemini", &["--version"]) {
        let agent = DetectedAgent::new(AgentKind::GeminiCli)
            .with_version(version)
            .with_detection_method(DetectionMethod::PathLookup);
        return Ok(Some(agent));
    }

    // Check for config directory
    if let Some(config_path) = AgentKind::GeminiCli.default_config_path()
        && config_path.parent().map(|p| p.exists()).unwrap_or(false)
    {
        let agent = DetectedAgent::new(AgentKind::GeminiCli)
            .with_detection_method(DetectionMethod::ConfigFile);
        return Ok(Some(agent));
    }

    Ok(None)
}

/// Detect Codex CLI installation.
fn detect_codex_cli() -> Result<Option<DetectedAgent>> {
    // Try to find codex in PATH
    if let Some(version) = get_command_version("codex", &["--version"]) {
        let agent = DetectedAgent::new(AgentKind::CodexCli)
            .with_version(version)
            .with_detection_method(DetectionMethod::PathLookup);
        return Ok(Some(agent));
    }

    // Check for config directory
    if let Some(config_path) = AgentKind::CodexCli.default_config_path()
        && config_path.parent().map(|p| p.exists()).unwrap_or(false)
    {
        let agent = DetectedAgent::new(AgentKind::CodexCli)
            .with_detection_method(DetectionMethod::ConfigFile);
        return Ok(Some(agent));
    }

    Ok(None)
}

/// Detect Hermes without loading or activating any plugin.
fn detect_hermes() -> Result<Option<DetectedAgent>> {
    if let Some(version) = get_command_version("hermes", &["--version"]) {
        return Ok(Some(
            DetectedAgent::new(AgentKind::Hermes)
                .with_version(version)
                .with_detection_method(DetectionMethod::PathLookup),
        ));
    }
    if let Some(home) = dirs::home_dir()
        && home.join(".hermes").is_dir()
    {
        return Ok(Some(
            DetectedAgent::new(AgentKind::Hermes)
                .with_detection_method(DetectionMethod::ConfigFile),
        ));
    }
    Ok(None)
}

/// Detect Cursor installation.
fn detect_cursor() -> Result<Option<DetectedAgent>> {
    // Check for Cursor config directory
    if let Some(config_path) = AgentKind::Cursor.default_config_path()
        && config_path.exists()
    {
        let agent = DetectedAgent::new(AgentKind::Cursor)
            .with_detection_method(DetectionMethod::ConfigFile);
        return Ok(Some(agent));
    }

    // Check common installation paths
    let cursor_paths = [
        "/usr/share/applications/cursor.desktop",
        "/opt/Cursor/cursor",
    ];

    for path in cursor_paths {
        if Path::new(path).exists() {
            let agent = DetectedAgent::new(AgentKind::Cursor)
                .with_detection_method(DetectionMethod::ConfigFile);
            return Ok(Some(agent));
        }
    }

    Ok(None)
}

/// Detect Continue.dev installation.
fn detect_continue_dev() -> Result<Option<DetectedAgent>> {
    // Check for Continue config directory
    if let Some(config_path) = AgentKind::ContinueDev.default_config_path()
        && config_path.exists()
    {
        let agent = DetectedAgent::new(AgentKind::ContinueDev)
            .with_detection_method(DetectionMethod::ConfigFile);
        return Ok(Some(agent));
    }

    Ok(None)
}

/// Detect Windsurf installation.
fn detect_windsurf() -> Result<Option<DetectedAgent>> {
    // Check for Windsurf config directory
    if let Some(config_path) = AgentKind::Windsurf.default_config_path()
        && config_path.exists()
    {
        let agent = DetectedAgent::new(AgentKind::Windsurf)
            .with_detection_method(DetectionMethod::ConfigFile);
        return Ok(Some(agent));
    }

    Ok(None)
}

/// Detect Aider installation.
fn detect_aider() -> Result<Option<DetectedAgent>> {
    // Try to find aider in PATH
    if let Some(version) = get_command_version("aider", &["--version"]) {
        let agent = DetectedAgent::new(AgentKind::Aider)
            .with_version(version)
            .with_detection_method(DetectionMethod::PathLookup);
        return Ok(Some(agent));
    }

    // Check for config file
    if let Some(config_path) = AgentKind::Aider.default_config_path()
        && config_path.exists()
    {
        let agent =
            DetectedAgent::new(AgentKind::Aider).with_detection_method(DetectionMethod::ConfigFile);
        return Ok(Some(agent));
    }

    Ok(None)
}

/// Detect Cline installation.
fn detect_cline() -> Result<Option<DetectedAgent>> {
    // Cline is a VS Code extension, check for its data directory
    if let Some(home) = dirs::home_dir() {
        let cline_path = home.join(".vscode").join("extensions");
        if cline_path.exists() {
            // Look for cline extension
            if let Ok(entries) = std::fs::read_dir(&cline_path) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("saoudrizwan.claude-dev")
                        || name_str.starts_with("cline.")
                    {
                        let agent = DetectedAgent::new(AgentKind::Cline)
                            .with_detection_method(DetectionMethod::ConfigFile);
                        return Ok(Some(agent));
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Get version from a command.
fn get_command_version(cmd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(cmd).args(args).output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Try stdout first, then stderr (some tools output version to stderr)
    let version_str = if !stdout.trim().is_empty() {
        stdout
    } else {
        stderr
    };

    // Extract version number from output
    extract_version(&version_str)
}

/// Extract version number from a version string.
fn extract_version(s: &str) -> Option<String> {
    // Common patterns:
    // "claude 1.0.0"
    // "v1.0.0"
    // "1.0.0"
    // "tool version 1.0.0"

    let s = s.trim();

    // Try to find a semver-like pattern
    for word in s.split_whitespace() {
        let word = word.trim_start_matches('v');
        if word
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            // Check if it looks like a version number
            if word.contains('.') || word.chars().all(|c| c.is_ascii_digit()) {
                return Some(word.to_string());
            }
        }
    }

    // Fallback: return the first line
    s.lines().next().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== TEST: extract_version =====

    #[test]
    fn test_extract_version_semver() {
        eprintln!("TEST START: test_extract_version_semver");
        assert_eq!(extract_version("1.0.0"), Some("1.0.0".to_string()));
        assert_eq!(extract_version("v1.0.0"), Some("1.0.0".to_string()));
        eprintln!("TEST PASS: test_extract_version_semver");
    }

    #[test]
    fn test_extract_version_with_prefix() {
        eprintln!("TEST START: test_extract_version_with_prefix");
        assert_eq!(extract_version("claude 1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(
            extract_version("tool version 2.0.0"),
            Some("2.0.0".to_string())
        );
        eprintln!("TEST PASS: test_extract_version_with_prefix");
    }

    #[test]
    fn test_extract_version_fallback() {
        eprintln!("TEST START: test_extract_version_fallback");
        assert_eq!(
            extract_version("unknown format"),
            Some("unknown format".to_string())
        );
        eprintln!("TEST PASS: test_extract_version_fallback");
    }

    #[test]
    fn test_extract_version_complex_formats() {
        eprintln!("TEST START: test_extract_version_complex_formats");

        // Test with build metadata
        assert_eq!(
            extract_version("v1.2.3-beta.1+build.456"),
            Some("1.2.3-beta.1+build.456".to_string())
        );

        // Test with prerelease
        assert_eq!(
            extract_version("version: 2.0.0-rc.1"),
            Some("2.0.0-rc.1".to_string())
        );

        // Test multiline (should take first line if no version found)
        let multiline = "Tool v1.2.3\nSome other info";
        assert_eq!(extract_version(multiline), Some("1.2.3".to_string()));

        eprintln!("TEST PASS: test_extract_version_complex_formats");
    }

    #[test]
    fn test_extract_version_edge_cases() {
        eprintln!("TEST START: test_extract_version_edge_cases");

        // Empty string
        assert_eq!(extract_version(""), None);

        // Whitespace only
        assert_eq!(extract_version("   "), None);

        // Just a number (not a version)
        assert_eq!(extract_version("42"), Some("42".to_string()));

        // Leading/trailing whitespace
        assert_eq!(extract_version("  v1.0.0  "), Some("1.0.0".to_string()));

        eprintln!("TEST PASS: test_extract_version_edge_cases");
    }

    #[test]
    fn test_extract_version_real_world_outputs() {
        eprintln!("TEST START: test_extract_version_real_world_outputs");

        // Simulated real-world version outputs
        assert_eq!(
            extract_version("claude-code version 1.0.51"),
            Some("1.0.51".to_string())
        );
        assert_eq!(extract_version("gemini 0.5.2"), Some("0.5.2".to_string()));
        assert_eq!(
            extract_version("codex v2.1.0-nightly"),
            Some("2.1.0-nightly".to_string())
        );
        assert_eq!(
            extract_version("aider --version\n0.42.0"),
            Some("0.42.0".to_string())
        );

        eprintln!("TEST PASS: test_extract_version_real_world_outputs");
    }

    // ===== TEST: detect_agents =====

    #[test]
    fn test_detect_agents_returns_vec() {
        eprintln!("TEST START: test_detect_agents_returns_vec");
        // This test just ensures the function runs without panicking
        let result = detect_agents();
        assert!(result.is_ok());
        eprintln!("  Detected {} agent(s)", result.as_ref().unwrap().len());
        for agent in result.unwrap() {
            eprintln!(
                "    - {:?} (version: {:?}, method: {:?})",
                agent.kind, agent.version, agent.detection_method
            );
        }
        eprintln!("TEST PASS: test_detect_agents_returns_vec");
    }

    #[test]
    fn test_detect_agents_iterates_all_kinds() {
        eprintln!("TEST START: test_detect_agents_iterates_all_kinds");

        // The function should check all agent kinds
        let agents = detect_agents().expect("detect_agents should not fail");

        // All detected agents should have valid kinds from AgentKind::ALL
        for agent in &agents {
            assert!(
                AgentKind::ALL.contains(&agent.kind),
                "Detected unknown agent kind: {:?}",
                agent.kind
            );
        }

        eprintln!("  All {} detected agents have valid kinds", agents.len());
        eprintln!("TEST PASS: test_detect_agents_iterates_all_kinds");
    }

    #[test]
    fn test_detect_agents_no_duplicates() {
        eprintln!("TEST START: test_detect_agents_no_duplicates");

        let agents = detect_agents().expect("detect_agents should not fail");
        let mut seen_kinds = std::collections::HashSet::new();

        for agent in &agents {
            assert!(
                seen_kinds.insert(agent.kind),
                "Duplicate agent kind detected: {:?}",
                agent.kind
            );
        }

        eprintln!("  No duplicate agent kinds in {} detections", agents.len());
        eprintln!("TEST PASS: test_detect_agents_no_duplicates");
    }

    // ===== TEST: detect_single_agent =====

    #[test]
    fn test_detect_single_agent_claude_code() {
        eprintln!("TEST START: test_detect_single_agent_claude_code");

        let result = detect_single_agent(AgentKind::ClaudeCode);
        assert!(result.is_ok(), "Detection should not error: {:?}", result);

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::ClaudeCode);
                assert!(
                    agent.detection_method == DetectionMethod::PathLookup
                        || agent.detection_method == DetectionMethod::ConfigFile,
                    "Unexpected detection method: {:?}",
                    agent.detection_method
                );
                eprintln!(
                    "  Claude Code detected: version={:?}, method={:?}",
                    agent.version, agent.detection_method
                );
            }
            None => {
                eprintln!("  Claude Code not installed (expected if not present)");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_claude_code");
    }

    #[test]
    fn test_detect_single_agent_gemini_cli() {
        eprintln!("TEST START: test_detect_single_agent_gemini_cli");

        let result = detect_single_agent(AgentKind::GeminiCli);
        assert!(result.is_ok());

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::GeminiCli);
                eprintln!(
                    "  Gemini CLI detected: version={:?}, method={:?}",
                    agent.version, agent.detection_method
                );
            }
            None => {
                eprintln!("  Gemini CLI not installed");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_gemini_cli");
    }

    #[test]
    fn test_detect_single_agent_codex_cli() {
        eprintln!("TEST START: test_detect_single_agent_codex_cli");

        let result = detect_single_agent(AgentKind::CodexCli);
        assert!(result.is_ok());

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::CodexCli);
                eprintln!(
                    "  Codex CLI detected: version={:?}, method={:?}",
                    agent.version, agent.detection_method
                );
            }
            None => {
                eprintln!("  Codex CLI not installed");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_codex_cli");
    }

    #[test]
    fn test_detect_single_agent_cursor() {
        eprintln!("TEST START: test_detect_single_agent_cursor");

        let result = detect_single_agent(AgentKind::Cursor);
        assert!(result.is_ok());

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::Cursor);
                // Cursor is detected via config file only
                assert_eq!(agent.detection_method, DetectionMethod::ConfigFile);
                eprintln!("  Cursor detected via config file");
            }
            None => {
                eprintln!("  Cursor not installed");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_cursor");
    }

    #[test]
    fn test_detect_single_agent_continue_dev() {
        eprintln!("TEST START: test_detect_single_agent_continue_dev");

        let result = detect_single_agent(AgentKind::ContinueDev);
        assert!(result.is_ok());

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::ContinueDev);
                eprintln!("  Continue.dev detected");
            }
            None => {
                eprintln!("  Continue.dev not installed");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_continue_dev");
    }

    #[test]
    fn test_detect_single_agent_windsurf() {
        eprintln!("TEST START: test_detect_single_agent_windsurf");

        let result = detect_single_agent(AgentKind::Windsurf);
        assert!(result.is_ok());

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::Windsurf);
                eprintln!("  Windsurf detected");
            }
            None => {
                eprintln!("  Windsurf not installed");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_windsurf");
    }

    #[test]
    fn test_detect_single_agent_aider() {
        eprintln!("TEST START: test_detect_single_agent_aider");

        let result = detect_single_agent(AgentKind::Aider);
        assert!(result.is_ok());

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::Aider);
                eprintln!(
                    "  Aider detected: version={:?}, method={:?}",
                    agent.version, agent.detection_method
                );
            }
            None => {
                eprintln!("  Aider not installed");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_aider");
    }

    #[test]
    fn test_detect_single_agent_cline() {
        eprintln!("TEST START: test_detect_single_agent_cline");

        let result = detect_single_agent(AgentKind::Cline);
        assert!(result.is_ok());

        match result.unwrap() {
            Some(agent) => {
                assert_eq!(agent.kind, AgentKind::Cline);
                // Cline is a VS Code extension
                assert_eq!(agent.detection_method, DetectionMethod::ConfigFile);
                eprintln!("  Cline VS Code extension detected");
            }
            None => {
                eprintln!("  Cline not installed");
            }
        }

        eprintln!("TEST PASS: test_detect_single_agent_cline");
    }

    // ===== TEST: detect_single_agent covers all kinds =====

    #[test]
    fn test_detect_single_agent_all_kinds_no_panic() {
        eprintln!("TEST START: test_detect_single_agent_all_kinds_no_panic");

        for kind in AgentKind::ALL {
            let result = detect_single_agent(*kind);
            assert!(
                result.is_ok(),
                "detect_single_agent({:?}) should not panic or error",
                kind
            );
            eprintln!("  {:?}: {:?}", kind, result.unwrap().is_some());
        }

        eprintln!("TEST PASS: test_detect_single_agent_all_kinds_no_panic");
    }

    // ===== TEST: DetectedAgent fields are populated correctly =====

    #[test]
    fn test_detected_agent_fields_consistency() {
        eprintln!("TEST START: test_detected_agent_fields_consistency");

        let agents = detect_agents().expect("detect_agents should not fail");

        for agent in &agents {
            // Kind should be valid
            assert!(AgentKind::ALL.contains(&agent.kind));

            // hook_support should match the kind's support level
            assert_eq!(
                agent.hook_support,
                agent.kind.hook_support(),
                "Hook support mismatch for {:?}",
                agent.kind
            );

            // Detection method should not be NotDetected for detected agents
            assert_ne!(
                agent.detection_method,
                DetectionMethod::NotDetected,
                "Detected agent {:?} has NotDetected method",
                agent.kind
            );

            eprintln!(
                "  {:?}: version={:?}, hook_support={:?}, method={:?}",
                agent.kind, agent.version, agent.hook_support, agent.detection_method
            );
        }

        eprintln!("TEST PASS: test_detected_agent_fields_consistency");
    }

    // ===== TEST: PATH lookup vs config file detection =====

    #[test]
    fn test_detection_method_semantics() {
        eprintln!("TEST START: test_detection_method_semantics");

        let agents = detect_agents().expect("detect_agents should not fail");

        for agent in &agents {
            match agent.detection_method {
                DetectionMethod::PathLookup => {
                    // PATH lookup should typically provide a version
                    eprintln!("  {:?} via PATH: version={:?}", agent.kind, agent.version);
                }
                DetectionMethod::ConfigFile => {
                    // Config file detection may not have a version
                    eprintln!("  {:?} via config: version={:?}", agent.kind, agent.version);
                }
                DetectionMethod::ProcessDetection => {
                    eprintln!(
                        "  {:?} via process: version={:?}",
                        agent.kind, agent.version
                    );
                }
                DetectionMethod::NotDetected => {
                    panic!("Should not have NotDetected for detected agent");
                }
            }
        }

        eprintln!("TEST PASS: test_detection_method_semantics");
    }
}
