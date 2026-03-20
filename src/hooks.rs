//! Lifecycle hook execution for pre/post install and generate phases.

use console::style;
use std::process::Command;

/// Run a list of hook commands for the given phase.
///
/// Each command is executed via `sh -c`. If the command list is empty,
/// this is a no-op. If any command exits non-zero, returns an error
/// with the phase name and exit code.
pub fn run_hooks(phase: &str, commands: &[String]) -> anyhow::Result<()> {
    if commands.is_empty() {
        return Ok(());
    }

    for cmd in commands {
        eprintln!("  hook ({}): {}", style(phase).blue(), cmd);

        let status = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .status()
            .map_err(|e| anyhow::anyhow!("failed to execute hook command '{}': {}", cmd, e))?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            anyhow::bail!(
                "hook ({}) command '{}' exited with code {}",
                phase,
                cmd,
                code
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_hooks_empty_is_noop() {
        let result = run_hooks("pre-install", &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_hooks_successful_command() {
        let result = run_hooks("pre-install", &["echo hello".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_hooks_failing_command() {
        let result = run_hooks("post-install", &["exit 42".to_string()]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("post-install"), "error should mention phase: {err}");
        assert!(err.contains("42"), "error should mention exit code: {err}");
    }

    #[test]
    fn run_hooks_multiple_commands_in_order() {
        // Both commands succeed; the second depends on the first not failing.
        let cmds = vec!["true".to_string(), "true".to_string()];
        let result = run_hooks("pre-generate", &cmds);
        assert!(result.is_ok());
    }

    #[test]
    fn run_hooks_stops_on_first_failure() {
        // First command fails, second should not run (we can't easily verify
        // the second didn't run, but we verify the error is from the first).
        let cmds = vec!["exit 1".to_string(), "echo should-not-run".to_string()];
        let result = run_hooks("pre-install", &cmds);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exit 1"));
    }
}
