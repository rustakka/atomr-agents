//! Command construction and execution for the guest agent. The argv builders
//! are pure (and unit-tested on every platform); the spawn path uses
//! `tokio::process` and runs inside the Linux guest.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use tokio::io::AsyncWriteExt;

use atomr_agents_sandbox_proto::Language;

/// Dependency-install argv for a language, if any deps are requested.
pub fn deps_command(language: Language, deps: &[String]) -> Option<(String, Vec<String>)> {
    if deps.is_empty() {
        return None;
    }
    match language {
        Language::Python => {
            let mut args = vec!["install".to_string(), "--quiet".to_string()];
            args.extend(deps.iter().cloned());
            Some(("pip".to_string(), args))
        }
        Language::Js => {
            let mut args = vec!["install".to_string(), "--silent".to_string()];
            args.extend(deps.iter().cloned());
            Some(("npm".to_string(), args))
        }
        Language::Rust | Language::Bash => None,
    }
}

/// Argv that runs the user code. Interpreted languages take the code as a
/// literal argv element (no shell, so no quoting hazard); Rust is base64-piped
/// to a file then compiled.
pub fn code_command(language: Language, code: &str, root: &Path) -> (String, Vec<String>) {
    match language {
        Language::Python => ("python".into(), vec!["-c".into(), code.into()]),
        Language::Bash => ("bash".into(), vec!["-c".into(), code.into()]),
        Language::Js => ("node".into(), vec!["-e".into(), code.into()]),
        Language::Rust => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(code);
            let dir = root.display();
            let script = format!(
                "echo {b64} | base64 -d > {dir}/main.rs && \
                 rustc {dir}/main.rs -o {dir}/main && {dir}/main"
            );
            ("sh".into(), vec!["-c".into(), script])
        }
    }
}

/// Captured output of one command run.
pub struct Captured {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
    pub timed_out: bool,
}

/// Run a command under `cwd`, capturing output, with an optional stdin and
/// wall-clock timeout.
pub async fn run_capture(
    cwd: &Path,
    program: &str,
    args: &[String],
    env: &HashMap<String, String>,
    stdin: Option<&str>,
    timeout: Option<Duration>,
) -> Result<Captured, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .envs(env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });

    let mut child = cmd.spawn().map_err(|e| format!("spawn {program}: {e}"))?;

    if let Some(input) = stdin {
        if let Some(mut sink) = child.stdin.take() {
            let _ = sink.write_all(input.as_bytes()).await;
            let _ = sink.shutdown().await;
        }
    }

    let wait = child.wait_with_output();
    let (output, timed_out) = match timeout {
        Some(d) => match tokio::time::timeout(d, wait).await {
            Ok(res) => (res.map_err(|e| format!("wait: {e}"))?, false),
            Err(_) => {
                return Ok(Captured {
                    stdout: Vec::new(),
                    stderr: b"timed out".to_vec(),
                    exit_code: 124,
                    timed_out: true,
                });
            }
        },
        None => (wait.await.map_err(|e| format!("wait: {e}"))?, false),
    };

    Ok(Captured {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.status.code().unwrap_or(-1),
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn python_code_command_passes_code_as_literal_argv() {
        let (prog, args) = code_command(Language::Python, "print(1)", &PathBuf::from("/w"));
        assert_eq!(prog, "python");
        assert_eq!(args, vec!["-c".to_string(), "print(1)".to_string()]);
    }

    #[test]
    fn js_and_bash_builders() {
        assert_eq!(code_command(Language::Js, "x", &PathBuf::from("/w")).0, "node");
        assert_eq!(code_command(Language::Bash, "x", &PathBuf::from("/w")).0, "bash");
    }

    #[test]
    fn rust_builder_base64_encodes_source() {
        let (prog, args) = code_command(Language::Rust, "fn main(){}", &PathBuf::from("/w"));
        assert_eq!(prog, "sh");
        assert!(args[1].contains("base64 -d > /w/main.rs"));
        assert!(args[1].contains("rustc"));
    }

    #[test]
    fn deps_commands_per_language() {
        let py = deps_command(Language::Python, &["requests".into()]).unwrap();
        assert_eq!(py.0, "pip");
        assert!(py.1.contains(&"requests".to_string()));
        assert_eq!(deps_command(Language::Js, &["left-pad".into()]).unwrap().0, "npm");
        assert!(deps_command(Language::Rust, &["serde".into()]).is_none());
        assert!(deps_command(Language::Python, &[]).is_none());
    }

    // Spawn test uses a shell available on unix CI/hosts; the guest runs Linux.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_capture_runs_a_real_command() {
        let cap = run_capture(
            &PathBuf::from("."),
            "sh",
            &["-c".into(), "printf hi; exit 7".into()],
            &HashMap::new(),
            None,
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert_eq!(cap.stdout, b"hi");
        assert_eq!(cap.exit_code, 7);
        assert!(!cap.timed_out);
    }
}
