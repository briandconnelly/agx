//! `agx list` — show the current dependency tree.

use anyhow::Context;
use clap::Args;
use console::style;

use agx::config;
use agx::manifest;

#[derive(Debug, Args)]
pub struct ListArgs {}

/// Run the `agx list` command.
pub fn run(_args: ListArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path = config::find_manifest(&cwd)
        .context("no agent-environment.yml found (run `agx init` first)")?;
    let env = manifest::load(&manifest_path)?;

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

            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", tags.join(", "))
            };
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
    Ok(())
}

/// Format the source description for a plugin dependency.
fn format_plugin_source(dep: &manifest::types::PluginDependency) -> String {
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
fn format_skill_source(dep: &manifest::types::SkillDependency) -> String {
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
fn format_agent_source(dep: &manifest::types::AgentDependency) -> String {
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
fn format_mcp_source(cfg: &manifest::types::McpServerConfig) -> String {
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
