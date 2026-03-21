//! Shared source-formatting helpers used by `list` and `graph` commands.

use agx::manifest::types::{AgentDependency, McpServerConfig, PluginDependency, SkillDependency};

/// Format the source description for a plugin dependency.
pub fn format_plugin_source(dep: &PluginDependency) -> String {
    if let Some(ref registry) = dep.registry {
        let version = dep.version.as_deref().unwrap_or("*");
        format!("{version} (registry: {registry})")
    } else if let Some(ref git) = dep.git {
        let ref_str = dep
            .tag
            .as_ref()
            .map(|t| format!("tag: {t}"))
            .or_else(|| dep.branch.as_ref().map(|b| format!("branch: {b}")))
            .or_else(|| dep.commit.as_ref().map(|c| format!("commit: {}", &c[..8.min(c.len())])))
            .unwrap_or_default();
        if ref_str.is_empty() {
            format!("(git: {git})")
        } else {
            format!("(git: {git}, {ref_str})")
        }
    } else {
        "(unknown source)".to_string()
    }
}

/// Format the source description for a skill dependency.
pub fn format_skill_source(dep: &SkillDependency) -> String {
    if dep.is_local() {
        format!("local: {}", dep.path)
    } else if let Some(ref git) = dep.git {
        let ref_str = dep
            .tag
            .as_ref()
            .map(|t| format!(", tag: {t}"))
            .or_else(|| dep.branch.as_ref().map(|b| format!(", branch: {b}")))
            .or_else(|| {
                dep.commit
                    .as_ref()
                    .map(|c| format!(", commit: {}", &c[..8.min(c.len())]))
            })
            .unwrap_or_default();
        format!("git: {git}{ref_str}")
    } else {
        format!("path: {}", dep.path)
    }
}

/// Format the source description for an agent dependency.
pub fn format_agent_source(dep: &AgentDependency) -> String {
    let mut parts = Vec::new();
    if dep.is_local() {
        parts.push(format!("local: {}", dep.path));
    } else if let Some(ref git) = dep.git {
        let ref_str = dep
            .tag
            .as_ref()
            .map(|t| format!(", tag: {t}"))
            .or_else(|| dep.branch.as_ref().map(|b| format!(", branch: {b}")))
            .or_else(|| {
                dep.commit
                    .as_ref()
                    .map(|c| format!(", commit: {}", &c[..8.min(c.len())]))
            })
            .unwrap_or_default();
        parts.push(format!("git: {git}{ref_str}"));
    } else {
        parts.push(format!("path: {}", dep.path));
    }
    if let Some(ref model) = dep.model {
        parts.push(format!("model: {model}"));
    }
    parts.join(", ")
}

/// Format the source description for an MCP server.
pub fn format_mcp_source(cfg: &McpServerConfig) -> String {
    if let Some(ref url) = cfg.url {
        format!("url: {url}")
    } else if let Some(ref cmd) = cfg.command {
        let args_str = if cfg.args.is_empty() {
            String::new()
        } else {
            format!(" {}", cfg.args.join(" "))
        };
        format!("command: {cmd}{args_str}")
    } else {
        "(unconfigured)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // ---- format_plugin_source ----

    #[test]
    fn plugin_source_registry() {
        let dep = PluginDependency {
            registry: Some("official".to_string()),
            version: Some(">=1.0.0".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let result = format_plugin_source(&dep);
        assert!(result.contains(">=1.0.0"));
        assert!(result.contains("registry: official"));
    }

    #[test]
    fn plugin_source_registry_no_version() {
        let dep = PluginDependency {
            registry: Some("official".to_string()),
            version: None,
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let result = format_plugin_source(&dep);
        assert!(result.contains("*"));
        assert!(result.contains("registry: official"));
    }

    #[test]
    fn plugin_source_git_tag() {
        let dep = PluginDependency {
            registry: None,
            version: None,
            git: Some("https://github.com/someone/plugin.git".to_string()),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let result = format_plugin_source(&dep);
        assert!(result.contains("git:"));
        assert!(result.contains("tag: v1.0.0"));
    }

    #[test]
    fn plugin_source_git_branch() {
        let dep = PluginDependency {
            registry: None,
            version: None,
            git: Some("https://github.com/someone/plugin.git".to_string()),
            tag: None,
            branch: Some("main".to_string()),
            commit: None,
            include: None,
            exclude: None,
        };
        let result = format_plugin_source(&dep);
        assert!(result.contains("branch: main"));
    }

    #[test]
    fn plugin_source_git_commit() {
        let dep = PluginDependency {
            registry: None,
            version: None,
            git: Some("https://github.com/someone/plugin.git".to_string()),
            tag: None,
            branch: None,
            commit: Some("abcdef1234567890".to_string()),
            include: None,
            exclude: None,
        };
        let result = format_plugin_source(&dep);
        assert!(result.contains("commit: abcdef12"));
    }

    #[test]
    fn plugin_source_git_no_ref() {
        let dep = PluginDependency {
            registry: None,
            version: None,
            git: Some("https://github.com/someone/plugin.git".to_string()),
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let result = format_plugin_source(&dep);
        assert_eq!(result, "(git: https://github.com/someone/plugin.git)");
    }

    #[test]
    fn plugin_source_unknown() {
        let dep = PluginDependency {
            registry: None,
            version: None,
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let result = format_plugin_source(&dep);
        assert_eq!(result, "(unknown source)");
    }

    // ---- format_skill_source ----

    #[test]
    fn skill_source_local() {
        let dep = SkillDependency {
            git: None,
            path: "./skills/my-skill".to_string(),
            tag: None,
            branch: None,
            commit: None,
        };
        let result = format_skill_source(&dep);
        assert_eq!(result, "local: ./skills/my-skill");
    }

    #[test]
    fn skill_source_git_tag() {
        let dep = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/x".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
        };
        let result = format_skill_source(&dep);
        assert!(result.contains("git:"));
        assert!(result.contains("tag: v1.0.0"));
    }

    #[test]
    fn skill_source_git_branch() {
        let dep = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/x".to_string(),
            tag: None,
            branch: Some("develop".to_string()),
            commit: None,
        };
        let result = format_skill_source(&dep);
        assert!(result.contains("branch: develop"));
    }

    #[test]
    fn skill_source_git_commit() {
        let dep = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/x".to_string(),
            tag: None,
            branch: None,
            commit: Some("abcdef1234567890".to_string()),
        };
        let result = format_skill_source(&dep);
        assert!(result.contains("commit: abcdef12"));
    }

    // ---- format_agent_source ----

    #[test]
    fn agent_source_local() {
        let dep = AgentDependency {
            git: None,
            path: "./agents/my-agent.md".to_string(),
            tag: None,
            branch: None,
            commit: None,
            model: None,
            parameters: Default::default(),
        };
        let result = format_agent_source(&dep);
        assert_eq!(result, "local: ./agents/my-agent.md");
    }

    #[test]
    fn agent_source_git_tag() {
        let dep = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/x.md".to_string(),
            tag: Some("v2.0.0".to_string()),
            branch: None,
            commit: None,
            model: None,
            parameters: Default::default(),
        };
        let result = format_agent_source(&dep);
        assert!(result.contains("git:"));
        assert!(result.contains("tag: v2.0.0"));
    }

    #[test]
    fn agent_source_local_with_model() {
        let dep = AgentDependency {
            git: None,
            path: "./agents/reviewer.md".to_string(),
            tag: None,
            branch: None,
            commit: None,
            model: Some("opus".to_string()),
            parameters: Default::default(),
        };
        let result = format_agent_source(&dep);
        assert!(result.contains("local: ./agents/reviewer.md"));
        assert!(result.contains("model: opus"));
    }

    #[test]
    fn agent_source_git_branch() {
        let dep = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/x.md".to_string(),
            tag: None,
            branch: Some("main".to_string()),
            commit: None,
            model: None,
            parameters: Default::default(),
        };
        let result = format_agent_source(&dep);
        assert!(result.contains("branch: main"));
    }

    #[test]
    fn agent_source_git_commit() {
        let dep = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/x.md".to_string(),
            tag: None,
            branch: None,
            commit: Some("abcdef1234567890".to_string()),
            model: None,
            parameters: Default::default(),
        };
        let result = format_agent_source(&dep);
        assert!(result.contains("commit: abcdef12"));
    }

    // ---- format_mcp_source ----

    #[test]
    fn mcp_source_url() {
        let cfg = McpServerConfig {
            url: Some("https://mcp.example.com/sse".to_string()),
            command: None,
            args: vec![],
            env: BTreeMap::new(),
        };
        let result = format_mcp_source(&cfg);
        assert_eq!(result, "url: https://mcp.example.com/sse");
    }

    #[test]
    fn mcp_source_command_with_args() {
        let cfg = McpServerConfig {
            url: None,
            command: Some("npx".to_string()),
            args: vec!["@example/mcp-server".to_string(), "--port".to_string(), "3000".to_string()],
            env: BTreeMap::new(),
        };
        let result = format_mcp_source(&cfg);
        assert_eq!(result, "command: npx @example/mcp-server --port 3000");
    }

    #[test]
    fn mcp_source_command_no_args() {
        let cfg = McpServerConfig {
            url: None,
            command: Some("mcp-server".to_string()),
            args: vec![],
            env: BTreeMap::new(),
        };
        let result = format_mcp_source(&cfg);
        assert_eq!(result, "command: mcp-server");
    }

    #[test]
    fn mcp_source_unconfigured() {
        let cfg = McpServerConfig {
            url: None,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
        };
        let result = format_mcp_source(&cfg);
        assert_eq!(result, "(unconfigured)");
    }
}
