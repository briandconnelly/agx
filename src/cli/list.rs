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
