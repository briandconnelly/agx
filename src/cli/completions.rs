//! Shell completion generation for agx.

use clap::CommandFactory;
use clap_complete::Shell;

/// Arguments for the `completions` subcommand.
#[derive(Debug, clap::Args)]
pub struct CompletionsArgs {
    /// The shell to generate completions for
    pub shell: Shell,
}

/// Generate shell completions and write them to stdout.
pub fn run(args: CompletionsArgs) -> anyhow::Result<()> {
    let mut cmd = super::Cli::command();
    clap_complete::generate(args.shell, &mut cmd, "agx", &mut std::io::stdout());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_non_empty_completions_for_all_shells() {
        let shells = [
            Shell::Bash,
            Shell::Zsh,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Elvish,
        ];
        for shell in shells {
            let mut cmd = super::super::Cli::command();
            let mut buf = Vec::new();
            clap_complete::generate(shell, &mut cmd, "agx", &mut buf);
            assert!(
                !buf.is_empty(),
                "completions for {shell:?} should not be empty"
            );
        }
    }
}
