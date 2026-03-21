//! `agx graph` — dependency graph visualization.

use anyhow::Context;
use clap::Args;
use console::style;

use agx::config;
use agx::lockfile;
use agx::manifest;
use agx::manifest::types::{
    AgentDependency, AgentEnvironment, BackendConfig, IsolationMode, McpServerConfig, ModelSpec,
    PluginDependency, SkillDependency,
};

use super::format_helpers::{format_agent_source, format_mcp_source, format_plugin_source, format_skill_source};

/// Show the dependency graph of the current project.
#[derive(Debug, Args)]
pub struct GraphArgs {
    /// Output format: tree (default), dot (Graphviz DOT)
    #[arg(long, default_value = "tree")]
    pub format: String,
}

/// Run the `agx graph` command.
pub fn run(args: GraphArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    let manifest_path =
        config::find_manifest(&cwd).context("no agent-environment.yml found (run `agx init` first)")?;
    let env = manifest::load(&manifest_path)?;

    // Try to load lockfile for resolved info
    let lockfile_path = manifest_path.with_file_name(config::LOCKFILE_FILE);
    let lock = if lockfile_path.exists() {
        lockfile::load(&lockfile_path).ok()
    } else {
        None
    };

    match args.format.as_str() {
        "tree" => print_tree(&env, lock.as_ref()),
        "dot" => print_dot(&env),
        other => anyhow::bail!("unknown format: {other} (expected 'tree' or 'dot')"),
    }

    Ok(())
}

/// Render the dependency graph as an indented tree (like `cargo tree`).
fn print_tree(env: &AgentEnvironment, lock: Option<&lockfile::types::Lockfile>) {
    let version_str = env
        .version
        .as_deref()
        .map(|v| format!(" v{v}"))
        .unwrap_or_default();
    println!("{}{}", style(&env.name).bold(), style(&version_str).dim());

    // Collect which sections exist so we know which is last.
    let mut sections: Vec<&str> = Vec::new();
    if !env.backends.is_empty() {
        sections.push("backends");
    }
    if !env.plugins.is_empty() {
        sections.push("plugins");
    }
    if !env.skills.is_empty() {
        sections.push("skills");
    }
    if !env.agents.is_empty() {
        sections.push("agents");
    }
    if !env.mcp_servers.is_empty() {
        sections.push("mcp-servers");
    }

    for (si, section) in sections.iter().enumerate() {
        let is_last_section = si == sections.len() - 1;
        let section_connector = if is_last_section { "└── " } else { "├── " };
        let section_cont = if is_last_section { "    " } else { "│   " };

        println!(
            "{}{}",
            style(section_connector).dim(),
            style(format!("[{section}]")).bold()
        );

        match *section {
            "backends" => {
                let items: Vec<_> = env.backends.iter().collect();
                print_section_items(&items, section_cont, |name, cfg| format_backend(name, cfg));
            }
            "plugins" => {
                let items: Vec<_> = env.plugins.iter().collect();
                print_section_items(&items, section_cont, |name, dep| {
                    format_plugin(name, dep, lock)
                });
            }
            "skills" => {
                let items: Vec<_> = env.skills.iter().collect();
                print_section_items(&items, section_cont, |name, dep| {
                    format_skill(name, dep, lock)
                });
            }
            "agents" => {
                let items: Vec<_> = env.agents.iter().collect();
                print_section_items(&items, section_cont, |name, dep| {
                    format_agent(name, dep, lock)
                });
            }
            "mcp-servers" => {
                let items: Vec<_> = env.mcp_servers.iter().collect();
                print_section_items(&items, section_cont, |name, cfg| format_mcp(name, cfg));
            }
            _ => {}
        }
    }
}

/// A formatted line for a tree item, with optional color info.
struct FormattedItem {
    /// The text to print (after the connector).
    text: String,
    /// Sub-items (e.g. plugin assets).
    children: Vec<String>,
}

/// Print items within a section, applying tree connectors.
fn print_section_items<T, F>(items: &[(&String, &T)], parent_prefix: &str, fmt: F)
where
    F: Fn(&str, &T) -> FormattedItem,
{
    for (i, (name, val)) in items.iter().enumerate() {
        let is_last = i == items.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        let item = fmt(name, val);
        println!(
            "{}{}{}",
            style(parent_prefix).dim(),
            style(connector).dim(),
            item.text
        );

        for (ci, child) in item.children.iter().enumerate() {
            let child_connector = if ci == item.children.len() - 1 {
                "└── "
            } else {
                "├── "
            };
            println!(
                "{}{}{}{}",
                style(parent_prefix).dim(),
                style(child_prefix).dim(),
                style(child_connector).dim(),
                child
            );
        }
    }
}

/// Format a backend entry.
fn format_backend(_name: &str, cfg: &BackendConfig) -> FormattedItem {
    let isolation = cfg
        .isolation
        .as_ref()
        .map(|i| match i {
            IsolationMode::Strict => "strict",
            IsolationMode::Permissive => "permissive",
        })
        .unwrap_or("strict");

    let model_str = cfg.model.as_ref().map(|m| match m {
        ModelSpec::Simple(s) => format!(", model: {s}"),
        ModelSpec::Full(f) => format!(", model: {}", f.id),
    }).unwrap_or_default();

    FormattedItem {
        text: format!("{_name} ({isolation}{model_str})"),
        children: Vec::new(),
    }
}

/// Format a plugin entry.
fn format_plugin(
    name: &str,
    dep: &PluginDependency,
    lock: Option<&lockfile::types::Lockfile>,
) -> FormattedItem {
    let source = format_plugin_source(dep);
    let resolved = lock
        .and_then(|l| l.plugins.get(name))
        .map(|lp| format_locked_info(&lp.source, lp.git_commit.as_deref()))
        .unwrap_or_default();

    let text = if resolved.is_empty() {
        format!("{name} {source}")
    } else {
        format!("{name} {source} {}", style(resolved).dim())
    };

    // Show included assets from the plugin (skills and agents from the filter).
    let mut children = Vec::new();
    if let Some(ref include) = dep.include {
        for s in &include.skills {
            children.push(format!("skill: {s}"));
        }
        for a in &include.agents {
            children.push(format!("agent: {a}"));
        }
    }

    FormattedItem { text, children }
}

/// Format a skill entry.
fn format_skill(
    name: &str,
    dep: &SkillDependency,
    lock: Option<&lockfile::types::Lockfile>,
) -> FormattedItem {
    let source = format_skill_source(dep);
    let colored_source = if dep.is_local() {
        style(source).green()
    } else {
        style(source).yellow()
    };

    let resolved = lock
        .and_then(|l| l.skills.get(name))
        .map(|ls| format_locked_info(&ls.source, ls.git_commit.as_deref()))
        .unwrap_or_default();

    let text = if resolved.is_empty() {
        format!("{name} ({colored_source})")
    } else {
        format!("{name} ({colored_source}) {}", style(resolved).dim())
    };

    FormattedItem {
        text,
        children: Vec::new(),
    }
}

/// Format an agent entry.
fn format_agent(
    name: &str,
    dep: &AgentDependency,
    lock: Option<&lockfile::types::Lockfile>,
) -> FormattedItem {
    let source = format_agent_source(dep);
    let colored_source = if dep.is_local() {
        style(source).green()
    } else {
        style(source).yellow()
    };

    let resolved = lock
        .and_then(|l| l.agents.get(name))
        .map(|la| format_locked_info(&la.source, la.git_commit.as_deref()))
        .unwrap_or_default();

    let text = if resolved.is_empty() {
        format!("{name} ({colored_source})")
    } else {
        format!("{name} ({colored_source}) {}", style(resolved).dim())
    };

    FormattedItem {
        text,
        children: Vec::new(),
    }
}

/// Format an MCP server entry.
fn format_mcp(_name: &str, cfg: &McpServerConfig) -> FormattedItem {
    let source = format_mcp_source(cfg);
    FormattedItem {
        text: format!("{_name} ({})", style(source).dim()),
        children: Vec::new(),
    }
}

/// Format locked/resolved info for display.
fn format_locked_info(source: &lockfile::types::LockedSource, commit: Option<&str>) -> String {
    use lockfile::types::LockedSource;
    let mut parts = Vec::new();
    match source {
        LockedSource::Registry { resolved_version, .. } => {
            parts.push(format!("resolved: {resolved_version}"));
        }
        LockedSource::Git { .. } => {}
        LockedSource::Npm { package, .. } => {
            parts.push(format!("npm: {package}"));
        }
        LockedSource::Local { .. } => {}
    }
    if let Some(sha) = commit {
        let short = &sha[..8.min(sha.len())];
        parts.push(format!("#{short}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("[{}]", parts.join(", "))
    }
}

/// Render the dependency graph in Graphviz DOT format.
fn print_dot(env: &AgentEnvironment) {
    let project_name = &env.name;
    println!("digraph \"{project_name}\" {{");
    println!("  rankdir=LR;");
    println!("  node [shape=box];");
    println!();
    println!("  root [label=\"{project_name}\"];");

    // Backends
    if !env.backends.is_empty() {
        println!();
        println!("  // Backends");
        for (name, cfg) in &env.backends {
            let node_id = dot_node_id("backend", name);
            let isolation = cfg
                .isolation
                .as_ref()
                .map(|i| match i {
                    IsolationMode::Strict => "strict",
                    IsolationMode::Permissive => "permissive",
                })
                .unwrap_or("strict");
            println!("  {node_id} [label=\"{name}\\n({isolation})\"];");
            println!("  root -> {node_id};");
        }
    }

    // Plugins
    if !env.plugins.is_empty() {
        println!();
        println!("  // Plugins");
        for (name, dep) in &env.plugins {
            let node_id = dot_node_id("plugin", name);
            let label = if let Some(ref version) = dep.version {
                format!("{name}\\n{version}")
            } else if dep.git.is_some() {
                format!("{name}\\n(git)")
            } else {
                name.to_string()
            };
            println!("  {node_id} [label=\"{label}\"];");
            println!("  root -> {node_id};");
        }
    }

    // Skills
    if !env.skills.is_empty() {
        println!();
        println!("  // Skills");
        for (name, dep) in &env.skills {
            let node_id = dot_node_id("skill", name);
            let label = if dep.is_local() {
                format!("{name}\\n(local)")
            } else {
                format!("{name}\\n(git)")
            };
            println!("  {node_id} [label=\"{label}\"];");
            println!("  root -> {node_id};");
        }
    }

    // Agents
    if !env.agents.is_empty() {
        println!();
        println!("  // Agents");
        for (name, dep) in &env.agents {
            let node_id = dot_node_id("agent", name);
            let label = if dep.is_local() {
                format!("{name}\\n(local)")
            } else {
                format!("{name}\\n(git)")
            };
            println!("  {node_id} [label=\"{label}\"];");
            println!("  root -> {node_id};");
        }
    }

    // MCP Servers
    if !env.mcp_servers.is_empty() {
        println!();
        println!("  // MCP Servers");
        for (name, cfg) in &env.mcp_servers {
            let node_id = dot_node_id("mcp", name);
            let label = if cfg.url.is_some() {
                format!("{name}\\n(url)")
            } else if cfg.command.is_some() {
                format!("{name}\\n(command)")
            } else {
                name.to_string()
            };
            println!("  {node_id} [label=\"{label}\"];");
            println!("  root -> {node_id};");
        }
    }

    println!("}}");
}

/// Create a valid DOT node identifier from a section prefix and asset name.
fn dot_node_id(prefix: &str, name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!("{prefix}_{sanitized}")
}

/// Build tree output as a string (for testing without colors).
#[cfg(test)]
fn build_tree_string(env: &AgentEnvironment) -> String {
    let mut out = String::new();

    let version_str = env
        .version
        .as_deref()
        .map(|v| format!(" v{v}"))
        .unwrap_or_default();
    out.push_str(&format!("{}{}\n", env.name, version_str));

    let mut sections: Vec<&str> = Vec::new();
    if !env.backends.is_empty() {
        sections.push("backends");
    }
    if !env.plugins.is_empty() {
        sections.push("plugins");
    }
    if !env.skills.is_empty() {
        sections.push("skills");
    }
    if !env.agents.is_empty() {
        sections.push("agents");
    }
    if !env.mcp_servers.is_empty() {
        sections.push("mcp-servers");
    }

    for (si, section) in sections.iter().enumerate() {
        let is_last_section = si == sections.len() - 1;
        let section_connector = if is_last_section { "└── " } else { "├── " };
        let section_cont = if is_last_section { "    " } else { "│   " };

        out.push_str(&format!("{section_connector}[{section}]\n"));

        match *section {
            "backends" => {
                let items: Vec<_> = env.backends.iter().collect();
                build_section_items_string(&mut out, &items, section_cont, |name, cfg| {
                    let isolation = cfg
                        .isolation
                        .as_ref()
                        .map(|i| match i {
                            IsolationMode::Strict => "strict",
                            IsolationMode::Permissive => "permissive",
                        })
                        .unwrap_or("strict");
                    let model_str = cfg.model.as_ref().map(|m| match m {
                        ModelSpec::Simple(s) => format!(", model: {s}"),
                        ModelSpec::Full(f) => format!(", model: {}", f.id),
                    }).unwrap_or_default();
                    (format!("{name} ({isolation}{model_str})"), Vec::new())
                });
            }
            "plugins" => {
                let items: Vec<_> = env.plugins.iter().collect();
                build_section_items_string(&mut out, &items, section_cont, |name, dep| {
                    let source = format_plugin_source(dep);
                    let mut children = Vec::new();
                    if let Some(ref include) = dep.include {
                        for s in &include.skills {
                            children.push(format!("skill: {s}"));
                        }
                        for a in &include.agents {
                            children.push(format!("agent: {a}"));
                        }
                    }
                    (format!("{name} {source}"), children)
                });
            }
            "skills" => {
                let items: Vec<_> = env.skills.iter().collect();
                build_section_items_string(&mut out, &items, section_cont, |name, dep| {
                    let source = format_skill_source(dep);
                    (format!("{name} ({source})"), Vec::new())
                });
            }
            "agents" => {
                let items: Vec<_> = env.agents.iter().collect();
                build_section_items_string(&mut out, &items, section_cont, |name, dep| {
                    let source = format_agent_source(dep);
                    (format!("{name} ({source})"), Vec::new())
                });
            }
            "mcp-servers" => {
                let items: Vec<_> = env.mcp_servers.iter().collect();
                build_section_items_string(&mut out, &items, section_cont, |name, cfg| {
                    let source = format_mcp_source(cfg);
                    (format!("{name} ({source})"), Vec::new())
                });
            }
            _ => {}
        }
    }

    out
}

/// Helper for building section items as strings (for testing).
#[cfg(test)]
fn build_section_items_string<T, F>(
    out: &mut String,
    items: &[(&String, &T)],
    parent_prefix: &str,
    fmt: F,
) where
    F: Fn(&str, &T) -> (String, Vec<String>),
{
    for (i, (name, val)) in items.iter().enumerate() {
        let is_last = i == items.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        let (text, children) = fmt(name, val);
        out.push_str(&format!("{parent_prefix}{connector}{text}\n"));

        for (ci, child) in children.iter().enumerate() {
            let child_connector = if ci == children.len() - 1 {
                "└── "
            } else {
                "├── "
            };
            out.push_str(&format!(
                "{parent_prefix}{child_prefix}{child_connector}{child}\n"
            ));
        }
    }
}

/// Build DOT output as a string (for testing).
#[cfg(test)]
fn build_dot_string(env: &AgentEnvironment) -> String {
    let mut out = String::new();
    let project_name = &env.name;
    out.push_str(&format!("digraph \"{project_name}\" {{\n"));
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [shape=box];\n");
    out.push_str("\n");
    out.push_str(&format!("  root [label=\"{project_name}\"];\n"));

    if !env.backends.is_empty() {
        out.push_str("\n  // Backends\n");
        for (name, cfg) in &env.backends {
            let node_id = dot_node_id("backend", name);
            let isolation = cfg
                .isolation
                .as_ref()
                .map(|i| match i {
                    IsolationMode::Strict => "strict",
                    IsolationMode::Permissive => "permissive",
                })
                .unwrap_or("strict");
            out.push_str(&format!("  {node_id} [label=\"{name}\\n({isolation})\"];\n"));
            out.push_str(&format!("  root -> {node_id};\n"));
        }
    }

    if !env.plugins.is_empty() {
        out.push_str("\n  // Plugins\n");
        for (name, dep) in &env.plugins {
            let node_id = dot_node_id("plugin", name);
            let label = if let Some(ref version) = dep.version {
                format!("{name}\\n{version}")
            } else if dep.git.is_some() {
                format!("{name}\\n(git)")
            } else {
                name.to_string()
            };
            out.push_str(&format!("  {node_id} [label=\"{label}\"];\n"));
            out.push_str(&format!("  root -> {node_id};\n"));
        }
    }

    if !env.skills.is_empty() {
        out.push_str("\n  // Skills\n");
        for (name, dep) in &env.skills {
            let node_id = dot_node_id("skill", name);
            let label = if dep.is_local() {
                format!("{name}\\n(local)")
            } else {
                format!("{name}\\n(git)")
            };
            out.push_str(&format!("  {node_id} [label=\"{label}\"];\n"));
            out.push_str(&format!("  root -> {node_id};\n"));
        }
    }

    if !env.agents.is_empty() {
        out.push_str("\n  // Agents\n");
        for (name, dep) in &env.agents {
            let node_id = dot_node_id("agent", name);
            let label = if dep.is_local() {
                format!("{name}\\n(local)")
            } else {
                format!("{name}\\n(git)")
            };
            out.push_str(&format!("  {node_id} [label=\"{label}\"];\n"));
            out.push_str(&format!("  root -> {node_id};\n"));
        }
    }

    if !env.mcp_servers.is_empty() {
        out.push_str("\n  // MCP Servers\n");
        for (name, cfg) in &env.mcp_servers {
            let node_id = dot_node_id("mcp", name);
            let label = if cfg.url.is_some() {
                format!("{name}\\n(url)")
            } else if cfg.command.is_some() {
                format!("{name}\\n(command)")
            } else {
                name.to_string()
            };
            out.push_str(&format!("  {node_id} [label=\"{label}\"];\n"));
            out.push_str(&format!("  root -> {node_id};\n"));
        }
    }

    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use agx::manifest::types::*;
    use std::collections::BTreeMap;

    /// Helper: create a minimal manifest with just a name and backends.
    fn minimal_manifest() -> AgentEnvironment {
        let mut backends = BTreeMap::new();
        backends.insert(
            "claude-code".to_string(),
            BackendConfig {
                default: Some(true),
                isolation: Some(IsolationMode::Strict),
                model: Some(ModelSpec::Simple("sonnet".to_string())),
            },
        );

        AgentEnvironment {
            name: "test-project".to_string(),
            version: Some("0.1.0".to_string()),
            description: None,
            backends,
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

    /// Helper: create a full manifest with all section types populated.
    fn full_manifest() -> AgentEnvironment {
        let mut env = minimal_manifest();
        env.backends.insert(
            "gemini-cli".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Permissive),
                model: None,
            },
        );

        // Plugins
        env.plugins.insert(
            "quality-review".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some(">=1.0.0,<2.0.0".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: Some(AssetFilter {
                    skills: vec!["code-review".to_string()],
                    agents: vec!["reviewer-bot".to_string()],
                }),
                exclude: None,
            },
        );
        env.plugins.insert(
            "custom-plugin".to_string(),
            PluginDependency {
                registry: None,
                version: None,
                git: Some("https://github.com/someone/their-plugin.git".to_string()),
                tag: Some("v3.0.0".to_string()),
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );

        // Skills
        env.skills.insert(
            "deploy-checklist".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/deploy-checklist".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        env.skills.insert(
            "remote-skill".to_string(),
            SkillDependency {
                git: Some("https://github.com/someone/skills.git".to_string()),
                path: "skills/remote".to_string(),
                tag: Some("v1.0.0".to_string()),
                branch: None,
                commit: None,
            },
        );

        // Agents
        env.agents.insert(
            "my-reviewer".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/my-reviewer.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: Some("opus".to_string()),
                parameters: BTreeMap::new(),
            },
        );
        env.agents.insert(
            "remote-agent".to_string(),
            AgentDependency {
                git: Some("https://github.com/someone/agents.git".to_string()),
                path: "agents/remote".to_string(),
                tag: Some("v2.0.0".to_string()),
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );

        // MCP Servers
        env.mcp_servers.insert(
            "playwright".to_string(),
            McpServerConfig {
                url: Some("https://mcp.playwright.dev/sse".to_string()),
                command: None,
                args: Vec::new(),
                env: BTreeMap::new(),
            },
        );
        env.mcp_servers.insert(
            "filesystem".to_string(),
            McpServerConfig {
                url: None,
                command: Some("npx".to_string()),
                args: vec!["@anthropic-ai/mcp-server-filesystem".to_string()],
                env: BTreeMap::new(),
            },
        );

        env
    }

    #[test]
    fn tree_output_contains_section_headers() {
        let env = full_manifest();
        let output = build_tree_string(&env);
        assert!(output.contains("[backends]"), "missing [backends] header");
        assert!(output.contains("[plugins]"), "missing [plugins] header");
        assert!(output.contains("[skills]"), "missing [skills] header");
        assert!(output.contains("[agents]"), "missing [agents] header");
        assert!(output.contains("[mcp-servers]"), "missing [mcp-servers] header");
    }

    #[test]
    fn tree_output_contains_asset_names() {
        let env = full_manifest();
        let output = build_tree_string(&env);
        assert!(output.contains("claude-code"), "missing claude-code");
        assert!(output.contains("gemini-cli"), "missing gemini-cli");
        assert!(output.contains("quality-review"), "missing quality-review");
        assert!(output.contains("custom-plugin"), "missing custom-plugin");
        assert!(output.contains("deploy-checklist"), "missing deploy-checklist");
        assert!(output.contains("remote-skill"), "missing remote-skill");
        assert!(output.contains("my-reviewer"), "missing my-reviewer");
        assert!(output.contains("remote-agent"), "missing remote-agent");
        assert!(output.contains("playwright"), "missing playwright");
        assert!(output.contains("filesystem"), "missing filesystem");
    }

    #[test]
    fn tree_output_contains_project_header() {
        let env = full_manifest();
        let output = build_tree_string(&env);
        assert!(output.starts_with("test-project v0.1.0\n"));
    }

    #[test]
    fn tree_output_with_empty_manifest() {
        let env = minimal_manifest();
        let output = build_tree_string(&env);
        assert!(output.contains("[backends]"));
        assert!(!output.contains("[plugins]"));
        assert!(!output.contains("[skills]"));
        assert!(!output.contains("[agents]"));
        assert!(!output.contains("[mcp-servers]"));
        // The only section should use └── (last section connector)
        assert!(output.contains("└── [backends]"));
    }

    #[test]
    fn tree_output_shows_plugin_children() {
        let env = full_manifest();
        let output = build_tree_string(&env);
        assert!(output.contains("skill: code-review"), "missing plugin skill child");
        assert!(output.contains("agent: reviewer-bot"), "missing plugin agent child");
    }

    #[test]
    fn tree_output_shows_backend_details() {
        let env = full_manifest();
        let output = build_tree_string(&env);
        assert!(output.contains("claude-code (strict, model: sonnet)"));
        assert!(output.contains("gemini-cli (permissive)"));
    }

    #[test]
    fn tree_output_shows_source_info() {
        let env = full_manifest();
        let output = build_tree_string(&env);
        assert!(output.contains("local: ./skills/deploy-checklist"), "missing local source");
        assert!(output.contains("registry: official"), "missing registry source");
        assert!(output.contains("git:"), "missing git source");
    }

    #[test]
    fn dot_output_is_valid_digraph() {
        let env = full_manifest();
        let output = build_dot_string(&env);
        assert!(output.starts_with("digraph \"test-project\""));
        assert!(output.contains("rankdir=LR"));
        assert!(output.contains("node [shape=box]"));
        assert!(output.contains("root [label=\"test-project\"]"));
        assert!(output.ends_with("}\n"));
    }

    #[test]
    fn dot_output_contains_expected_nodes() {
        let env = full_manifest();
        let output = build_dot_string(&env);
        assert!(output.contains("plugin_quality_review"), "missing plugin node");
        assert!(output.contains("plugin_custom_plugin"), "missing plugin node");
        assert!(output.contains("skill_deploy_checklist"), "missing skill node");
        assert!(output.contains("agent_my_reviewer"), "missing agent node");
        assert!(output.contains("mcp_playwright"), "missing mcp node");
        assert!(output.contains("mcp_filesystem"), "missing mcp node");
    }

    #[test]
    fn dot_output_contains_edges() {
        let env = full_manifest();
        let output = build_dot_string(&env);
        assert!(output.contains("root -> plugin_quality_review"));
        assert!(output.contains("root -> skill_deploy_checklist"));
        assert!(output.contains("root -> agent_my_reviewer"));
        assert!(output.contains("root -> mcp_playwright"));
    }

    #[test]
    fn dot_output_with_empty_manifest() {
        let env = minimal_manifest();
        let output = build_dot_string(&env);
        assert!(output.contains("digraph"));
        assert!(output.contains("// Backends"));
        assert!(output.contains("backend_claude_code"));
        assert!(!output.contains("// Plugins"));
        assert!(!output.contains("// Skills"));
    }

    #[test]
    fn dot_node_id_sanitizes_names() {
        assert_eq!(dot_node_id("plugin", "quality-review"), "plugin_quality_review");
        assert_eq!(dot_node_id("mcp", "my.server"), "mcp_my_server");
        assert_eq!(dot_node_id("skill", "foo bar"), "skill_foo_bar");
    }

    // -----------------------------------------------------------------------
    // format_locked_info tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_locked_info_registry_without_commit() {
        let source = lockfile::types::LockedSource::Registry {
            registry: "official".to_string(),
            resolved_version: "1.2.3".to_string(),
            path: None,
        };
        let result = format_locked_info(&source, None);
        assert_eq!(result, "[resolved: 1.2.3]");
    }

    #[test]
    fn format_locked_info_registry_with_path_and_commit() {
        let source = lockfile::types::LockedSource::Registry {
            registry: "official".to_string(),
            resolved_version: "2.0.0".to_string(),
            path: Some("plugins/foo".to_string()),
        };
        let result = format_locked_info(&source, Some("abcdef1234567890"));
        assert_eq!(result, "[resolved: 2.0.0, #abcdef12]");
    }

    #[test]
    fn format_locked_info_git_without_commit() {
        let source = lockfile::types::LockedSource::Git {
            git: "https://github.com/someone/repo.git".to_string(),
            path: None,
        };
        let result = format_locked_info(&source, None);
        assert_eq!(result, "");
    }

    #[test]
    fn format_locked_info_git_with_commit() {
        let source = lockfile::types::LockedSource::Git {
            git: "https://github.com/someone/repo.git".to_string(),
            path: Some("subdir".to_string()),
        };
        let result = format_locked_info(&source, Some("1234567890abcdef"));
        assert_eq!(result, "[#12345678]");
    }

    #[test]
    fn format_locked_info_git_with_short_commit() {
        let source = lockfile::types::LockedSource::Git {
            git: "https://github.com/someone/repo.git".to_string(),
            path: None,
        };
        let result = format_locked_info(&source, Some("abc"));
        assert_eq!(result, "[#abc]");
    }

    #[test]
    fn format_locked_info_npm_with_resolved_version() {
        let source = lockfile::types::LockedSource::Npm {
            package: "@scope/my-plugin".to_string(),
            resolved_version: Some("3.1.0".to_string()),
            npm_registry: None,
        };
        let result = format_locked_info(&source, None);
        assert_eq!(result, "[npm: @scope/my-plugin]");
    }

    #[test]
    fn format_locked_info_npm_without_resolved_version() {
        let source = lockfile::types::LockedSource::Npm {
            package: "simple-pkg".to_string(),
            resolved_version: None,
            npm_registry: None,
        };
        let result = format_locked_info(&source, None);
        assert_eq!(result, "[npm: simple-pkg]");
    }

    #[test]
    fn format_locked_info_npm_with_commit() {
        let source = lockfile::types::LockedSource::Npm {
            package: "pkg".to_string(),
            resolved_version: Some("1.0.0".to_string()),
            npm_registry: Some("https://registry.npmjs.org".to_string()),
        };
        let result = format_locked_info(&source, Some("deadbeef12345678"));
        assert_eq!(result, "[npm: pkg, #deadbeef]");
    }

    #[test]
    fn format_locked_info_local() {
        let source = lockfile::types::LockedSource::Local {
            local: "./my/local/path".to_string(),
        };
        let result = format_locked_info(&source, None);
        assert_eq!(result, "");
    }

    #[test]
    fn format_locked_info_local_with_commit() {
        let source = lockfile::types::LockedSource::Local {
            local: "./local".to_string(),
        };
        let result = format_locked_info(&source, Some("abcdef1234567890"));
        assert_eq!(result, "[#abcdef12]");
    }

    // -----------------------------------------------------------------------
    // More build_tree_string tests
    // -----------------------------------------------------------------------

    #[test]
    fn tree_output_mcp_with_command_and_args() {
        let mut env = minimal_manifest();
        env.mcp_servers.insert(
            "my-mcp".to_string(),
            McpServerConfig {
                url: None,
                command: Some("npx".to_string()),
                args: vec!["@scope/server".to_string(), "--port".to_string(), "3000".to_string()],
                env: BTreeMap::new(),
            },
        );
        let output = build_tree_string(&env);
        assert!(output.contains("[mcp-servers]"));
        assert!(output.contains("my-mcp"));
    }

    #[test]
    fn tree_output_mcp_with_url() {
        let mut env = minimal_manifest();
        env.mcp_servers.insert(
            "url-mcp".to_string(),
            McpServerConfig {
                url: Some("https://mcp.example.com/sse".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let output = build_tree_string(&env);
        assert!(output.contains("url-mcp"));
    }

    #[test]
    fn tree_output_only_skills() {
        let mut env = AgentEnvironment {
            name: "skills-only".to_string(),
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
        };
        env.skills.insert(
            "only-skill".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/only".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        let output = build_tree_string(&env);
        assert!(output.starts_with("skills-only\n"));
        assert!(output.contains("[skills]"));
        assert!(output.contains("only-skill"));
        assert!(!output.contains("[backends]"));
        assert!(!output.contains("[plugins]"));
    }

    #[test]
    fn tree_output_only_agents() {
        let mut env = AgentEnvironment {
            name: "agents-only".to_string(),
            version: Some("2.0.0".to_string()),
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
        };
        env.agents.insert(
            "solo-agent".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/solo.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        let output = build_tree_string(&env);
        assert!(output.contains("[agents]"));
        assert!(output.contains("solo-agent"));
        // Should use last-section connector
        assert!(output.contains("└── [agents]"));
    }

    #[test]
    fn tree_output_no_version() {
        let env = AgentEnvironment {
            name: "no-version".to_string(),
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
        };
        let output = build_tree_string(&env);
        assert!(output.starts_with("no-version\n"));
        // No version string appended
        assert!(!output.contains(" v"));
    }

    #[test]
    fn tree_output_completely_empty_manifest() {
        let env = AgentEnvironment {
            name: "empty".to_string(),
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
        };
        let output = build_tree_string(&env);
        // Just the project name, no sections
        assert_eq!(output.trim(), "empty");
    }

    // -----------------------------------------------------------------------
    // More build_dot_string tests
    // -----------------------------------------------------------------------

    #[test]
    fn dot_output_special_characters_in_names() {
        let mut env = AgentEnvironment {
            name: "special-chars".to_string(),
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
        };
        env.plugins.insert(
            "my.plugin@v2".to_string(),
            PluginDependency {
                registry: Some("r".to_string()),
                version: Some("2.0.0".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let output = build_dot_string(&env);
        // Special chars should be sanitized in node id
        assert!(output.contains("plugin_my_plugin_v2"));
        // But label should contain the original name
        assert!(output.contains("my.plugin@v2"));
    }

    #[test]
    fn dot_output_with_only_mcp_servers() {
        let mut env = AgentEnvironment {
            name: "mcp-only".to_string(),
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
        };
        env.mcp_servers.insert(
            "srv".to_string(),
            McpServerConfig {
                url: None,
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("// MCP Servers"));
        assert!(output.contains("mcp_srv"));
        // Label should be just the name (no url or command)
        assert!(output.contains("label=\"srv\""));
        assert!(!output.contains("// Backends"));
        assert!(!output.contains("// Plugins"));
    }

    #[test]
    fn dot_output_git_plugin_label() {
        let mut env = AgentEnvironment {
            name: "git-test".to_string(),
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
        };
        env.plugins.insert(
            "git-plugin".to_string(),
            PluginDependency {
                registry: None,
                version: None,
                git: Some("https://github.com/someone/plugin.git".to_string()),
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("git-plugin\\n(git)"));
    }

    #[test]
    fn dot_output_plugin_no_version_no_git() {
        let mut env = AgentEnvironment {
            name: "bare-test".to_string(),
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
        };
        env.plugins.insert(
            "bare-plugin".to_string(),
            PluginDependency {
                registry: None,
                version: None,
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("label=\"bare-plugin\""));
    }

    #[test]
    fn dot_output_agents_local_and_git() {
        let mut env = AgentEnvironment {
            name: "agent-test".to_string(),
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
        };
        env.agents.insert(
            "local-agent".to_string(),
            AgentDependency {
                git: None,
                path: "./agents/local.md".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        env.agents.insert(
            "git-agent".to_string(),
            AgentDependency {
                git: Some("https://github.com/someone/agents.git".to_string()),
                path: "agents/remote".to_string(),
                tag: None,
                branch: None,
                commit: None,
                model: None,
                parameters: BTreeMap::new(),
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("local-agent\\n(local)"));
        assert!(output.contains("git-agent\\n(git)"));
    }

    #[test]
    fn dot_output_skills_local_and_git() {
        let mut env = AgentEnvironment {
            name: "skill-test".to_string(),
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
        };
        env.skills.insert(
            "local-skill".to_string(),
            SkillDependency {
                git: None,
                path: "./skills/local".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        env.skills.insert(
            "git-skill".to_string(),
            SkillDependency {
                git: Some("https://github.com/someone/skills.git".to_string()),
                path: "skills/remote".to_string(),
                tag: None,
                branch: None,
                commit: None,
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("local-skill\\n(local)"));
        assert!(output.contains("git-skill\\n(git)"));
    }

    // -----------------------------------------------------------------------
    // format_backend tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_backend_strict_with_simple_model() {
        let cfg = BackendConfig {
            default: Some(true),
            isolation: Some(IsolationMode::Strict),
            model: Some(ModelSpec::Simple("sonnet".to_string())),
        };
        let item = format_backend("claude-code", &cfg);
        assert_eq!(item.text, "claude-code (strict, model: sonnet)");
        assert!(item.children.is_empty());
    }

    #[test]
    fn format_backend_permissive_no_model() {
        let cfg = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Permissive),
            model: None,
        };
        let item = format_backend("gemini-cli", &cfg);
        assert_eq!(item.text, "gemini-cli (permissive)");
        assert!(item.children.is_empty());
    }

    #[test]
    fn format_backend_no_isolation_defaults_strict() {
        let cfg = BackendConfig {
            default: None,
            isolation: None,
            model: None,
        };
        let item = format_backend("generic", &cfg);
        assert_eq!(item.text, "generic (strict)");
    }

    #[test]
    fn format_backend_with_full_model() {
        let cfg = BackendConfig {
            default: None,
            isolation: Some(IsolationMode::Strict),
            model: Some(ModelSpec::Full(ModelConfig {
                id: "claude-opus-4-6".to_string(),
                extended_context: None,
                effort: None,
                plan_model: None,
                available: Vec::new(),
                parameters: BTreeMap::new(),
            })),
        };
        let item = format_backend("claude-code", &cfg);
        assert_eq!(item.text, "claude-code (strict, model: claude-opus-4-6)");
    }

    // -----------------------------------------------------------------------
    // format_plugin tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_plugin_registry_no_lock() {
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
        let item = format_plugin("my-plugin", &dep, None);
        assert!(item.text.contains("my-plugin"));
        assert!(item.text.contains("registry: official"));
        assert!(item.children.is_empty());
    }

    #[test]
    fn format_plugin_with_include_filter() {
        let dep = PluginDependency {
            registry: Some("official".to_string()),
            version: Some("^1".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: Some(AssetFilter {
                skills: vec!["skill-a".to_string(), "skill-b".to_string()],
                agents: vec!["agent-x".to_string()],
            }),
            exclude: None,
        };
        let item = format_plugin("filtered", &dep, None);
        assert_eq!(item.children.len(), 3);
        assert_eq!(item.children[0], "skill: skill-a");
        assert_eq!(item.children[1], "skill: skill-b");
        assert_eq!(item.children[2], "agent: agent-x");
    }

    #[test]
    fn format_plugin_with_lockfile_resolved() {
        let dep = PluginDependency {
            registry: Some("official".to_string()),
            version: Some("^1".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let mut lock = lockfile::types::Lockfile::new();
        lock.plugins.insert(
            "resolved-plugin".to_string(),
            lockfile::types::LockedEntry {
                source: lockfile::types::LockedSource::Registry {
                    registry: "official".to_string(),
                    resolved_version: "1.2.3".to_string(),
                    path: None,
                },
                git_commit: Some("abcdef1234567890".to_string()),
                integrity: None,
            },
        );
        let item = format_plugin("resolved-plugin", &dep, Some(&lock));
        // Should contain resolved info in the text
        assert!(item.text.contains("resolved: 1.2.3"));
        assert!(item.text.contains("#abcdef12"));
    }

    #[test]
    fn format_plugin_lockfile_no_matching_entry() {
        let dep = PluginDependency {
            registry: Some("official".to_string()),
            version: Some("^1".to_string()),
            git: None,
            tag: None,
            branch: None,
            commit: None,
            include: None,
            exclude: None,
        };
        let lock = lockfile::types::Lockfile::new();
        let item = format_plugin("unlocked-plugin", &dep, Some(&lock));
        // No resolved info, so no brackets
        assert!(!item.text.contains('['));
    }

    // -----------------------------------------------------------------------
    // format_skill tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_skill_local_no_lock() {
        let dep = SkillDependency {
            git: None,
            path: "./skills/my-skill".to_string(),
            tag: None,
            branch: None,
            commit: None,
        };
        let item = format_skill("my-skill", &dep, None);
        assert!(item.text.contains("my-skill"));
        assert!(item.text.contains("local:"));
        assert!(item.children.is_empty());
    }

    #[test]
    fn format_skill_git_no_lock() {
        let dep = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/remote".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            commit: None,
        };
        let item = format_skill("remote-skill", &dep, None);
        assert!(item.text.contains("remote-skill"));
        assert!(item.text.contains("git:"));
    }

    #[test]
    fn format_skill_with_lockfile_resolved() {
        let dep = SkillDependency {
            git: Some("https://github.com/someone/skills.git".to_string()),
            path: "skills/remote".to_string(),
            tag: None,
            branch: None,
            commit: None,
        };
        let mut lock = lockfile::types::Lockfile::new();
        lock.skills.insert(
            "locked-skill".to_string(),
            lockfile::types::LockedEntry {
                source: lockfile::types::LockedSource::Git {
                    git: "https://github.com/someone/skills.git".to_string(),
                    path: Some("skills/remote".to_string()),
                },
                git_commit: Some("1234567890abcdef".to_string()),
                integrity: None,
            },
        );
        let item = format_skill("locked-skill", &dep, Some(&lock));
        assert!(item.text.contains("#12345678"));
    }

    // -----------------------------------------------------------------------
    // format_agent tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_agent_local_no_lock() {
        let dep = AgentDependency {
            git: None,
            path: "./agents/my-agent.md".to_string(),
            tag: None,
            branch: None,
            commit: None,
            model: Some("opus".to_string()),
            parameters: BTreeMap::new(),
        };
        let item = format_agent("my-agent", &dep, None);
        assert!(item.text.contains("my-agent"));
        assert!(item.text.contains("local:"));
        assert!(item.children.is_empty());
    }

    #[test]
    fn format_agent_git_no_lock() {
        let dep = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/remote".to_string(),
            tag: Some("v2.0.0".to_string()),
            branch: None,
            commit: None,
            model: None,
            parameters: BTreeMap::new(),
        };
        let item = format_agent("remote-agent", &dep, None);
        assert!(item.text.contains("remote-agent"));
        assert!(item.text.contains("git:"));
    }

    #[test]
    fn format_agent_with_lockfile_resolved() {
        let dep = AgentDependency {
            git: Some("https://github.com/someone/agents.git".to_string()),
            path: "agents/remote".to_string(),
            tag: None,
            branch: None,
            commit: None,
            model: None,
            parameters: BTreeMap::new(),
        };
        let mut lock = lockfile::types::Lockfile::new();
        lock.agents.insert(
            "locked-agent".to_string(),
            lockfile::types::LockedEntry {
                source: lockfile::types::LockedSource::Git {
                    git: "https://github.com/someone/agents.git".to_string(),
                    path: Some("agents/remote".to_string()),
                },
                git_commit: Some("deadbeef12345678".to_string()),
                integrity: None,
            },
        );
        let item = format_agent("locked-agent", &dep, Some(&lock));
        assert!(item.text.contains("#deadbeef"));
    }

    // -----------------------------------------------------------------------
    // format_mcp tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_mcp_url_source() {
        let cfg = McpServerConfig {
            url: Some("https://mcp.example.com/sse".to_string()),
            command: None,
            args: vec![],
            env: BTreeMap::new(),
        };
        let item = format_mcp("my-mcp", &cfg);
        assert!(item.text.contains("my-mcp"));
        assert!(item.text.contains("url:"));
        assert!(item.children.is_empty());
    }

    #[test]
    fn format_mcp_command_source() {
        let cfg = McpServerConfig {
            url: None,
            command: Some("npx".to_string()),
            args: vec!["@scope/server".to_string()],
            env: BTreeMap::new(),
        };
        let item = format_mcp("cmd-mcp", &cfg);
        assert!(item.text.contains("cmd-mcp"));
        assert!(item.text.contains("command:"));
    }

    #[test]
    fn format_mcp_unconfigured() {
        let cfg = McpServerConfig {
            url: None,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
        };
        let item = format_mcp("bare-mcp", &cfg);
        assert!(item.text.contains("bare-mcp"));
        assert!(item.text.contains("unconfigured"));
    }

    // -----------------------------------------------------------------------
    // build_tree_string with lockfile-related content
    // -----------------------------------------------------------------------

    #[test]
    fn tree_output_multiple_sections_correct_connectors() {
        // When there are exactly two sections, the first uses ├── and the last uses └──
        let mut env = AgentEnvironment {
            name: "two-sections".to_string(),
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
        };
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
        env.mcp_servers.insert(
            "m1".to_string(),
            McpServerConfig {
                url: Some("https://example.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let output = build_tree_string(&env);
        assert!(output.contains("├── [skills]"));
        assert!(output.contains("└── [mcp-servers]"));
    }

    #[test]
    fn tree_output_multiple_items_in_section_correct_connectors() {
        let mut env = AgentEnvironment {
            name: "multi-items".to_string(),
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
        };
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
        let output = build_tree_string(&env);
        // First two items use ├──, last uses └──
        let lines: Vec<&str> = output.lines().collect();
        let skill_lines: Vec<&&str> = lines.iter().filter(|l| l.contains("alpha") || l.contains("beta") || l.contains("gamma")).collect();
        assert_eq!(skill_lines.len(), 3);
        assert!(skill_lines[0].contains("├──"));
        assert!(skill_lines[1].contains("├──"));
        assert!(skill_lines[2].contains("└──"));
    }

    #[test]
    fn tree_output_plugin_multiple_children_connectors() {
        let mut env = AgentEnvironment {
            name: "children-test".to_string(),
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
        };
        env.plugins.insert(
            "big-plugin".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some("^1".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: Some(AssetFilter {
                    skills: vec!["s1".to_string(), "s2".to_string()],
                    agents: vec!["a1".to_string()],
                }),
                exclude: None,
            },
        );
        let output = build_tree_string(&env);
        assert!(output.contains("skill: s1"));
        assert!(output.contains("skill: s2"));
        assert!(output.contains("agent: a1"));
        // Verify child connectors: first two use ├──, last uses └──
        let lines: Vec<&str> = output.lines().collect();
        let child_lines: Vec<&&str> = lines.iter().filter(|l| l.contains("skill: s") || l.contains("agent: a1")).collect();
        assert_eq!(child_lines.len(), 3);
        assert!(child_lines[0].contains("├──"));
        assert!(child_lines[1].contains("├──"));
        assert!(child_lines[2].contains("└──"));
    }

    // -----------------------------------------------------------------------
    // dot_output additional edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn dot_output_completely_empty_manifest() {
        let env = AgentEnvironment {
            name: "empty-dot".to_string(),
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
        };
        let output = build_dot_string(&env);
        assert!(output.contains("digraph \"empty-dot\""));
        assert!(output.contains("root [label=\"empty-dot\"]"));
        assert!(!output.contains("// Backends"));
        assert!(!output.contains("// Plugins"));
        assert!(!output.contains("// Skills"));
        assert!(!output.contains("// Agents"));
        assert!(!output.contains("// MCP Servers"));
    }

    #[test]
    fn dot_output_backend_permissive_vs_strict() {
        let mut env = AgentEnvironment {
            name: "isolation-test".to_string(),
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
        };
        env.backends.insert(
            "strict-be".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Strict),
                model: None,
            },
        );
        env.backends.insert(
            "perm-be".to_string(),
            BackendConfig {
                default: None,
                isolation: Some(IsolationMode::Permissive),
                model: None,
            },
        );
        env.backends.insert(
            "default-be".to_string(),
            BackendConfig {
                default: None,
                isolation: None,
                model: None,
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("(strict)"));
        assert!(output.contains("(permissive)"));
        // default-be has no isolation set, defaults to strict
        let default_line = output.lines().find(|l| l.contains("default-be")).unwrap();
        assert!(default_line.contains("strict"));
    }

    #[test]
    fn dot_output_plugin_with_version() {
        let mut env = AgentEnvironment {
            name: "version-test".to_string(),
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
        };
        env.plugins.insert(
            "versioned".to_string(),
            PluginDependency {
                registry: Some("official".to_string()),
                version: Some(">=2.0.0".to_string()),
                git: None,
                tag: None,
                branch: None,
                commit: None,
                include: None,
                exclude: None,
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("versioned\\n>=2.0.0"));
    }

    #[test]
    fn dot_node_id_all_alphanumeric() {
        assert_eq!(dot_node_id("x", "abc123"), "x_abc123");
    }

    #[test]
    fn dot_node_id_empty_name() {
        assert_eq!(dot_node_id("plugin", ""), "plugin_");
    }

    // -----------------------------------------------------------------------
    // run() integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn run_tree_format_succeeds_with_manifest() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_content = "name: graph-test\nbackends:\n  claude-code:\n    default: true\n    isolation: strict\nskills:\n  my-skill:\n    path: ./skills/my-skill\n";
        std::fs::write(dir.path().join("agent-environment.yml"), manifest_content).unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = GraphArgs { format: "tree".to_string() };
        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_ok());
    }

    #[test]
    fn run_dot_format_succeeds_with_manifest() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_content = "name: graph-dot-test\nbackends:\n  claude-code:\n    default: true\n    isolation: strict\nplugins:\n  my-plugin:\n    registry: official\n    version: \">=1.0.0\"\n";
        std::fs::write(dir.path().join("agent-environment.yml"), manifest_content).unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = GraphArgs { format: "dot".to_string() };
        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_ok());
    }

    #[test]
    fn run_unknown_format_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_content = "name: graph-err\nbackends:\n  claude-code: {}\n";
        std::fs::write(dir.path().join("agent-environment.yml"), manifest_content).unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = GraphArgs { format: "invalid".to_string() };
        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown format"));
    }

    #[test]
    fn run_no_manifest_errors() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = GraphArgs { format: "tree".to_string() };
        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_err());
    }

    #[test]
    fn run_tree_with_lockfile() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_content = "name: lock-graph\nbackends:\n  claude-code:\n    default: true\nskills:\n  my-skill:\n    path: ./skills/my-skill\n";
        std::fs::write(dir.path().join("agent-environment.yml"), manifest_content).unwrap();

        // Create a lockfile
        let mut lock = lockfile::types::Lockfile::new();
        lock.skills.insert(
            "my-skill".to_string(),
            lockfile::types::LockedEntry {
                source: lockfile::types::LockedSource::Local {
                    local: "./skills/my-skill".to_string(),
                },
                git_commit: None,
                integrity: None,
            },
        );
        let lock_content = serde_yaml::to_string(&lock).unwrap();
        std::fs::write(dir.path().join("agent-environment.lock"), &lock_content).unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = GraphArgs { format: "tree".to_string() };
        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_ok());
    }

    #[test]
    fn run_tree_full_manifest_all_sections() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_content = r#"
name: full-graph
version: "1.0.0"
backends:
  claude-code:
    default: true
    isolation: strict
  gemini-cli:
    isolation: permissive
plugins:
  my-plugin:
    registry: official
    version: ">=1.0.0"
    include:
      skills:
        - skill-a
      agents:
        - agent-b
skills:
  local-skill:
    path: ./skills/local
  remote-skill:
    git: https://github.com/test/skills.git
    path: skills/remote
    tag: v1.0.0
agents:
  local-agent:
    path: ./agents/local.md
  remote-agent:
    git: https://github.com/test/agents.git
    path: agents/remote
mcp-servers:
  url-mcp:
    url: https://example.com/sse
  cmd-mcp:
    command: npx
    args:
      - "@scope/server"
"#;
        std::fs::write(dir.path().join("agent-environment.yml"), manifest_content).unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = GraphArgs { format: "tree".to_string() };
        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_ok());
    }

    #[test]
    fn run_dot_full_manifest_all_sections() {
        let _guard = super::super::PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let manifest_content = r#"
name: full-dot
backends:
  claude-code:
    default: true
plugins:
  git-plugin:
    git: https://github.com/test/plugin.git
skills:
  local-skill:
    path: ./skills/local
agents:
  git-agent:
    git: https://github.com/test/agents.git
    path: agents/remote
mcp-servers:
  bare-mcp: {}
"#;
        std::fs::write(dir.path().join("agent-environment.yml"), manifest_content).unwrap();

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let args = GraphArgs { format: "dot".to_string() };
        let result = run(args);
        std::env::set_current_dir(&old_cwd).unwrap();

        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Additional edge case tests
    // -----------------------------------------------------------------------

    #[test]
    fn dot_output_mcp_url_vs_command() {
        let mut env = AgentEnvironment {
            name: "mcp-test".to_string(),
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
        };
        env.mcp_servers.insert(
            "url-srv".to_string(),
            McpServerConfig {
                url: Some("https://example.com".to_string()),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        env.mcp_servers.insert(
            "cmd-srv".to_string(),
            McpServerConfig {
                url: None,
                command: Some("node".to_string()),
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let output = build_dot_string(&env);
        assert!(output.contains("url-srv\\n(url)"));
        assert!(output.contains("cmd-srv\\n(command)"));
    }
}
