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

// --- Source formatting (duplicated from list.rs per task instructions) ---

/// Format the source description for a plugin dependency.
fn format_plugin_source(dep: &PluginDependency) -> String {
    if let Some(ref registry) = dep.registry {
        let version = dep.version.as_deref().unwrap_or("*");
        format!("{version} ({})", style(format!("registry: {registry}")).cyan())
    } else if let Some(ref git) = dep.git {
        let ref_str = dep
            .tag
            .as_ref()
            .map(|t| format!("tag: {t}"))
            .or_else(|| dep.branch.as_ref().map(|b| format!("branch: {b}")))
            .or_else(|| {
                dep.commit
                    .as_ref()
                    .map(|c| format!("commit: {}", &c[..8.min(c.len())]))
            })
            .unwrap_or_default();
        let git_info = if ref_str.is_empty() {
            format!("git: {git}")
        } else {
            format!("git: {git}, {ref_str}")
        };
        format!("({})", style(git_info).yellow())
    } else {
        "(unknown source)".to_string()
    }
}

/// Format the source description for a skill dependency.
fn format_skill_source(dep: &SkillDependency) -> String {
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
fn format_agent_source(dep: &AgentDependency) -> String {
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
fn format_mcp_source(cfg: &McpServerConfig) -> String {
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
                    let source = if let Some(ref registry) = dep.registry {
                        let version = dep.version.as_deref().unwrap_or("*");
                        format!("{version} (registry: {registry})")
                    } else if let Some(ref git) = dep.git {
                        let ref_str = dep
                            .tag
                            .as_ref()
                            .map(|t| format!("tag: {t}"))
                            .or_else(|| dep.branch.as_ref().map(|b| format!("branch: {b}")))
                            .or_else(|| {
                                dep.commit
                                    .as_ref()
                                    .map(|c| format!("commit: {}", &c[..8.min(c.len())]))
                            })
                            .unwrap_or_default();
                        if ref_str.is_empty() {
                            format!("(git: {git})")
                        } else {
                            format!("(git: {git}, {ref_str})")
                        }
                    } else {
                        "(unknown source)".to_string()
                    };
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
                    let source = if dep.is_local() {
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
                    };
                    (format!("{name} ({source})"), Vec::new())
                });
            }
            "agents" => {
                let items: Vec<_> = env.agents.iter().collect();
                build_section_items_string(&mut out, &items, section_cont, |name, dep| {
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
                    (format!("{name} ({})", parts.join(", ")), Vec::new())
                });
            }
            "mcp-servers" => {
                let items: Vec<_> = env.mcp_servers.iter().collect();
                build_section_items_string(&mut out, &items, section_cont, |name, cfg| {
                    let source = if let Some(ref url) = cfg.url {
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
                    };
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
}
