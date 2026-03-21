//! `agx list` — show the current dependency tree.

use anyhow::Context;
use clap::Args;
use console::style;

use agx::config;
use agx::manifest;

use super::format_helpers::{format_agent_source, format_mcp_source, format_plugin_source, format_skill_source};

#[derive(Debug, Args)]
pub struct ListArgs {}

/// Run the `agx list` command.
pub fn run(_args: ListArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let env = manifest::load(&manifest_path)?;

    print_listing(&env);
    Ok(())
}

/// Format a backend's tag string (e.g. "(default, strict)").
fn format_backend_tags(cfg: &agx::manifest::types::BackendConfig) -> String {
    let mut tags = Vec::new();
    if cfg.default == Some(true) {
        tags.push("default".to_string());
    }
    let isolation = cfg
        .isolation
        .as_ref()
        .map(|i| format!("{i:?}").to_lowercase())
        .unwrap_or_else(|| "strict".to_string());
    tags.push(isolation);

    if tags.is_empty() {
        String::new()
    } else {
        format!(" ({})", tags.join(", "))
    }
}

/// Build the list of output lines for an `AgentEnvironment`.
///
/// This is the core display logic, separated from I/O so it can be tested.
pub fn build_listing(env: &agx::manifest::types::AgentEnvironment) -> Vec<String> {
    let mut lines = Vec::new();

    // Header
    let version_str = env
        .version
        .as_deref()
        .map(|v| format!(" v{v}"))
        .unwrap_or_default();
    lines.push(format!("{}{}", &env.name, &version_str));

    if let Some(ref desc) = env.description {
        lines.push(desc.clone());
    }

    // Backends
    if !env.backends.is_empty() {
        lines.push(String::new());
        lines.push("Backends:".to_string());
        for (name, cfg) in &env.backends {
            let tag_str = format_backend_tags(cfg);
            lines.push(format!("  ● {name}{tag_str}"));
        }
    }

    // Plugins
    if !env.plugins.is_empty() {
        lines.push(String::new());
        lines.push("Plugins:".to_string());
        let items: Vec<_> = env.plugins.iter().collect();
        for (i, (name, dep)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_plugin_source(dep);
            lines.push(format!("  {connector} {name} {source}"));
        }
    }

    // Skills
    if !env.skills.is_empty() {
        lines.push(String::new());
        lines.push("Skills:".to_string());
        let items: Vec<_> = env.skills.iter().collect();
        for (i, (name, dep)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_skill_source(dep);
            lines.push(format!("  {connector} {name} ({source})"));
        }
    }

    // Agents
    if !env.agents.is_empty() {
        lines.push(String::new());
        lines.push("Agents:".to_string());
        let items: Vec<_> = env.agents.iter().collect();
        for (i, (name, dep)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_agent_source(dep);
            lines.push(format!("  {connector} {name} ({source})"));
        }
    }

    // MCP Servers
    if !env.mcp_servers.is_empty() {
        lines.push(String::new());
        lines.push("MCP Servers:".to_string());
        let items: Vec<_> = env.mcp_servers.iter().collect();
        for (i, (name, cfg)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_mcp_source(cfg);
            lines.push(format!("  {connector} {name} ({source})"));
        }
    }

    lines.push(String::new());
    lines
}

/// Print the listing to stdout with styling.
fn print_listing(env: &agx::manifest::types::AgentEnvironment) {
    // Header
    let version_str = env
        .version
        .as_deref()
        .map(|v| format!(" v{v}"))
        .unwrap_or_default();
    println!(
        "{}{}",
        style(&env.name).bold(),
        style(&version_str).dim()
    );

    if let Some(ref desc) = env.description {
        println!("{}", style(desc).dim());
    }

    // Backends
    if !env.backends.is_empty() {
        println!();
        println!("{}", style("Backends:").bold());
        for (name, cfg) in &env.backends {
            let tag_str = format_backend_tags(cfg);
            println!("  {} {}{}", style("●").green(), name, style(tag_str).dim());
        }
    }

    // Plugins
    if !env.plugins.is_empty() {
        println!();
        println!("{}", style("Plugins:").bold());
        let items: Vec<_> = env.plugins.iter().collect();
        for (i, (name, dep)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_plugin_source(dep);
            println!("  {connector} {name} {}", style(source).dim());
        }
    }

    // Skills
    if !env.skills.is_empty() {
        println!();
        println!("{}", style("Skills:").bold());
        let items: Vec<_> = env.skills.iter().collect();
        for (i, (name, dep)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_skill_source(dep);
            let colored_source = if dep.is_local() {
                style(source).green()
            } else {
                style(source).yellow()
            };
            println!("  {connector} {name} ({colored_source})");
        }
    }

    // Agents
    if !env.agents.is_empty() {
        println!();
        println!("{}", style("Agents:").bold());
        let items: Vec<_> = env.agents.iter().collect();
        for (i, (name, dep)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_agent_source(dep);
            let colored_source = if dep.is_local() {
                style(source).green()
            } else {
                style(source).yellow()
            };
            println!("  {connector} {name} ({colored_source})");
        }
    }

    // MCP Servers
    if !env.mcp_servers.is_empty() {
        println!();
        println!("{}", style("MCP Servers:").bold());
        let items: Vec<_> = env.mcp_servers.iter().collect();
        for (i, (name, cfg)) in items.iter().enumerate() {
            let connector = if i == items.len() - 1 { "└──" } else { "├──" };
            let source = format_mcp_source(cfg);
            println!("  {connector} {name} ({})", style(source).dim());
        }
    }

    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    fn minimal_env(name: &str) -> AgentEnvironment {
        AgentEnvironment {
            name: name.to_string(),
            version: None,
            description: None,
            backends: BTreeMap::new(),
            registries: BTreeMap::new(),
            plugins: BTreeMap::new(),
            skills: BTreeMap::new(),
            agents: BTreeMap::new(),
            mcp_servers: BTreeMap::new(),
            settings: BTreeMap::new(),
            paths: None,
            context: None,
            hooks: None,
        }
    }

    #[test]
    fn build_listing_minimal() {
        let env = minimal_env("test-project");
        let lines = build_listing(&env);
        assert_eq!(lines[0], "test-project");
        // Should end with an empty line
        assert_eq!(lines.last().unwrap(), "");
        // No sections printed for empty maps
        assert!(!lines.iter().any(|l| l.contains("Backends:")));
        assert!(!lines.iter().any(|l| l.contains("Plugins:")));
    }

    #[test]
    fn build_listing_with_version_and_description() {
        let mut env = minimal_env("my-project");
        env.version = Some("1.2.3".to_string());
        env.description = Some("A cool project".to_string());
        let lines = build_listing(&env);
        assert_eq!(lines[0], "my-project v1.2.3");
        assert_eq!(lines[1], "A cool project");
    }

    #[test]
    fn build_listing_with_backends() {
        let mut env = minimal_env("test");
        env.backends.insert(
            "claude-code".to_string(),
            BackendConfig {
                default: Some(true),
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );
        env.backends.insert(
            "gemini-cli".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Permissive),
                model: None,
            },
        );
        let lines = build_listing(&env);
        assert!(lines.iter().any(|l| l.contains("Backends:")));
        assert!(lines.iter().any(|l| l.contains("claude-code") && l.contains("default, strict")));
        assert!(lines.iter().any(|l| l.contains("gemini-cli") && l.contains("permissive")));
    }

    #[test]
    fn build_listing_with_plugins() {
        let mut env = minimal_env("test");
        env.plugins.insert(
            "quality-review".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some(">=1.0.0".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let lines = build_listing(&env);
        assert!(lines.iter().any(|l| l.contains("Plugins:")));
        // Single item uses └──
        assert!(lines
            .iter()
            .any(|l| l.contains("└──") && l.contains("quality-review")));
    }

    #[test]
    fn build_listing_with_skills() {
        let mut env = minimal_env("test");
        env.skills.insert(
            "deploy".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/deploy".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        let lines = build_listing(&env);
        assert!(lines.iter().any(|l| l.contains("Skills:")));
        assert!(lines
            .iter()
            .any(|l| l.contains("└──") && l.contains("deploy")));
    }

    #[test]
    fn build_listing_with_agents() {
        let mut env = minimal_env("test");
        env.agents.insert(
            "reviewer".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/reviewer.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("opus".to_string()),
                parameters: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        assert!(lines.iter().any(|l| l.contains("Agents:")));
        assert!(lines
            .iter()
            .any(|l| l.contains("└──") && l.contains("reviewer")));
    }

    #[test]
    fn build_listing_with_mcp_servers() {
        let mut env = minimal_env("test");
        env.mcp_servers.insert(
            "playwright".to_string(),
            McpServerConfig {
                url: Some("https://mcp.playwright.dev/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        assert!(lines.iter().any(|l| l.contains("MCP Servers:")));
        assert!(lines
            .iter()
            .any(|l| l.contains("└──") && l.contains("playwright")));
    }

    #[test]
    fn build_listing_full_manifest() {
        let mut env = minimal_env("full-project");
        env.version = Some("0.1.0".to_string());
        env.description = Some("Full test".to_string());
        env.backends.insert(
            "claude-code".to_string(),
            BackendConfig {
                default: Some(true),
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );
        env.plugins.insert(
            "p1".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some("^1".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        env.skills.insert(
            "s1".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/s1".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        env.agents.insert(
            "a1".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/a1.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        env.mcp_servers.insert(
            "m1".to_string(),
            McpServerConfig {
                url: Some("https://example.com/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );

        let lines = build_listing(&env);
        assert_eq!(lines[0], "full-project v0.1.0");
        assert_eq!(lines[1], "Full test");
        assert!(lines.iter().any(|l| l.contains("Backends:")));
        assert!(lines.iter().any(|l| l.contains("Plugins:")));
        assert!(lines.iter().any(|l| l.contains("Skills:")));
        assert!(lines.iter().any(|l| l.contains("Agents:")));
        assert!(lines.iter().any(|l| l.contains("MCP Servers:")));
    }

    #[test]
    fn build_listing_multiple_plugins_uses_tree_connectors() {
        let mut env = minimal_env("test");
        env.plugins.insert(
            "alpha".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: None,
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        env.plugins.insert(
            "beta".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: None,
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let lines = build_listing(&env);
        let plugin_lines: Vec<_> = lines
            .iter()
            .filter(|l| l.contains("├──") || l.contains("└──"))
            .collect();
        assert_eq!(plugin_lines.len(), 2);
        assert!(plugin_lines[0].contains("├──"));
        assert!(plugin_lines[1].contains("└──"));
    }

    #[test]
    fn format_backend_tags_default_strict() {
        let cfg = BackendConfig {
            default: Some(true),
            isolation: Some(IsolationMode::Strict),
            model: None,
        };
        let tags = format_backend_tags(&cfg);
        assert_eq!(tags, " (default, strict)");
    }

    #[test]
    fn format_backend_tags_no_default_no_isolation() {
        let cfg = BackendConfig {
            default: None,
            isolation: None,
            model: None,
        };
        let tags = format_backend_tags(&cfg);
        // Falls back to "strict" when isolation is None
        assert_eq!(tags, " (strict)");
    }

    #[test]
    fn format_backend_tags_permissive() {
        let cfg = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Permissive),
            model: None,
        };
        let tags = format_backend_tags(&cfg);
        assert_eq!(tags, " (permissive)");
    }

    #[test]
    fn format_backend_tags_default_false_not_shown() {
        let cfg = BackendConfig {
            default: Some(false),
            isolation: Some(IsolationMode::Strict),
            model: None,
        };
        let tags = format_backend_tags(&cfg);
        // default: Some(false) should NOT show "default"
        assert_eq!(tags, " (strict)");
    }

    // -----------------------------------------------------------------------
    // build_listing edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn build_listing_no_version_no_description() {
        let env = minimal_env("bare-project");
        let lines = build_listing(&env);
        assert_eq!(lines[0], "bare-project");
        // No description line — next should be empty trailing line
        assert_eq!(lines[1], "");
    }

    #[test]
    fn build_listing_skills_local_source_format() {
        let mut env = minimal_env("test");
        env.skills.insert(
            "local-skill".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/local-skill".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        let lines = build_listing(&env);
        let skill_line = lines.iter().find(|l| l.contains("local-skill")).unwrap();
        assert!(skill_line.contains("local: ./skills/local-skill"));
    }

    #[test]
    fn build_listing_skills_git_source_format() {
        let mut env = minimal_env("test");
        env.skills.insert(
            "git-skill".to_string(),
            SkillDependency {
                git: Some("https://github.com/someone/skills.git".to_string()),
                path: "skills/remote".to_string(),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
            },
        );
        let lines = build_listing(&env);
        let skill_line = lines.iter().find(|l| l.contains("git-skill")).unwrap();
        assert!(skill_line.contains("git:"));
        assert!(skill_line.contains("tag: v1.0.0"));
    }

    #[test]
    fn build_listing_agents_local_source_format() {
        let mut env = minimal_env("test");
        env.agents.insert(
            "local-agent".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/local-agent.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("opus".to_string()),
                parameters: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        let agent_line = lines.iter().find(|l| l.contains("local-agent")).unwrap();
        assert!(agent_line.contains("local: ./agents/local-agent.md"));
        assert!(agent_line.contains("model: opus"));
    }

    #[test]
    fn build_listing_agents_git_source_format() {
        let mut env = minimal_env("test");
        env.agents.insert(
            "git-agent".to_string(),
            AgentDependency {
                git: Some("https://github.com/someone/agents.git".to_string()),
                path: "agents/remote".to_string(),
                tag: None,
                branch: Some("main".to_string()),
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        let agent_line = lines.iter().find(|l| l.contains("git-agent")).unwrap();
        assert!(agent_line.contains("git:"));
        assert!(agent_line.contains("branch: main"));
    }

    #[test]
    fn build_listing_mcp_url_source_format() {
        let mut env = minimal_env("test");
        env.mcp_servers.insert(
            "url-mcp".to_string(),
            McpServerConfig {
                url: Some("https://mcp.example.com/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        let mcp_line = lines.iter().find(|l| l.contains("url-mcp")).unwrap();
        assert!(mcp_line.contains("url: https://mcp.example.com/sse"));
    }

    #[test]
    fn build_listing_mcp_command_source_format() {
        let mut env = minimal_env("test");
        env.mcp_servers.insert(
            "cmd-mcp".to_string(),
            McpServerConfig {
                url: None,
                command: Some("npx".to_string()),
                args: vec!["@scope/server".to_string()],
                env: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        let mcp_line = lines.iter().find(|l| l.contains("cmd-mcp")).unwrap();
        assert!(mcp_line.contains("command: npx @scope/server"));
    }

    #[test]
    fn build_listing_multiple_skills_connectors() {
        let mut env = minimal_env("test");
        env.skills.insert(
            "alpha".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/alpha".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        env.skills.insert(
            "beta".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/beta".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        env.skills.insert(
            "gamma".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/gamma".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        let lines = build_listing(&env);
        let skill_lines: Vec<_> = lines.iter().filter(|l| l.contains("├──") || l.contains("└──")).collect();
        assert_eq!(skill_lines.len(), 3);
        assert!(skill_lines[0].contains("├──") && skill_lines[0].contains("alpha"));
        assert!(skill_lines[1].contains("├──") && skill_lines[1].contains("beta"));
        assert!(skill_lines[2].contains("└──") && skill_lines[2].contains("gamma"));
    }

    #[test]
    fn build_listing_multiple_agents_connectors() {
        let mut env = minimal_env("test");
        env.agents.insert(
            "agent-a".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/a.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        env.agents.insert(
            "agent-b".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/b.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        let agent_lines: Vec<_> = lines.iter().filter(|l| l.contains("agent-")).collect();
        assert_eq!(agent_lines.len(), 2);
        assert!(agent_lines[0].contains("├──"));
        assert!(agent_lines[1].contains("└──"));
    }

    #[test]
    fn build_listing_multiple_mcp_connectors() {
        let mut env = minimal_env("test");
        env.mcp_servers.insert(
            "mcp-a".to_string(),
            McpServerConfig {
                url: Some("https://a.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        env.mcp_servers.insert(
            "mcp-b".to_string(),
            McpServerConfig {
                url: Some("https://b.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        let mcp_lines: Vec<_> = lines.iter().filter(|l| l.contains("mcp-")).collect();
        assert_eq!(mcp_lines.len(), 2);
        assert!(mcp_lines[0].contains("├──"));
        assert!(mcp_lines[1].contains("└──"));
    }

    #[test]
    fn build_listing_plugin_git_source() {
        let mut env = minimal_env("test");
        env.plugins.insert(
            "git-plugin".to_string(),
            PluginDependency {
                registry: None,
                version: None,
                git: Some("https://github.com/someone/plugin.git".to_string()),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let lines = build_listing(&env);
        let plugin_line = lines.iter().find(|l| l.contains("git-plugin")).unwrap();
        assert!(plugin_line.contains("git:"));
        assert!(plugin_line.contains("tag: v1.0.0"));
    }

    #[test]
    fn build_listing_all_sections_present() {
        let mut env = minimal_env("all-sections");
        env.version = Some("3.0.0".to_string());
        env.description = Some("Everything".to_string());
        env.backends.insert(
            "claude-code".to_string(),
            BackendConfig {
                default: Some(true),
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );
        env.plugins.insert(
            "p1".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: Some("^1".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        env.skills.insert(
            "s1".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/s1".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        env.agents.insert(
            "a1".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/a1.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        env.mcp_servers.insert(
            "m1".to_string(),
            McpServerConfig {
                url: Some("https://example.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        assert_eq!(lines[0], "all-sections v3.0.0");
        assert_eq!(lines[1], "Everything");
        assert!(lines.iter().any(|l| l == "Backends:"));
        assert!(lines.iter().any(|l| l == "Plugins:"));
        assert!(lines.iter().any(|l| l == "Skills:"));
        assert!(lines.iter().any(|l| l == "Agents:"));
        assert!(lines.iter().any(|l| l == "MCP Servers:"));
    }

    #[test]
    fn build_listing_ends_with_empty_line() {
        let env = minimal_env("test");
        let lines = build_listing(&env);
        assert_eq!(lines.last().unwrap(), "");
    }

    #[test]
    fn build_listing_mcp_unconfigured() {
        let mut env = minimal_env("test");
        env.mcp_servers.insert(
            "bare-mcp".to_string(),
            McpServerConfig {
                url: None,
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let lines = build_listing(&env);
        let mcp_line = lines.iter().find(|l| l.contains("bare-mcp")).unwrap();
        assert!(mcp_line.contains("(unconfigured)"));
    }
}
