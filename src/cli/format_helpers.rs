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
