use super::error::CommandError;

fn ensure_command_succeeded(
    program: &str,
    output: &std::process::Output,
) -> Result<(), CommandError> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    let message = if detail.is_empty() {
        format!("{program} exited with status {}", output.status)
    } else {
        format!("{program} failed: {detail}")
    };

    Err(CommandError::Internal { message })
}

#[tauri::command]
#[specta::specta]
pub async fn clipboard_paste(text: String, auto_paste: bool) -> Result<(), CommandError> {
    // Write to clipboard via pbcopy (macOS)
    #[cfg(target_os = "macos")]
    {
        let mut child = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| CommandError::Internal {
                message: format!("pbcopy: {e}"),
            })?;
        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| CommandError::Internal {
                    message: e.to_string(),
                })?;
        }
        child.wait().map_err(|e| CommandError::Internal {
            message: e.to_string(),
        })?;
    }

    #[cfg(target_os = "windows")]
    {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "clip"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| CommandError::Internal {
                message: format!("clip: {e}"),
            })?;
        if let Some(ref mut stdin) = child.stdin {
            use std::io::Write;
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| CommandError::Internal {
                    message: e.to_string(),
                })?;
        }
        child.wait().map_err(|e| CommandError::Internal {
            message: e.to_string(),
        })?;
    }

    if auto_paste {
        // Brief delay for clipboard to settle
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("osascript")
                .args([
                    "-e",
                    r#"tell application "System Events" to keystroke "v" using command down"#,
                ])
                .output()
                .map_err(|e| CommandError::Internal {
                    message: format!("osascript: {e}"),
                })?;
            ensure_command_succeeded("osascript", &output)?;
        }

        #[cfg(target_os = "windows")]
        {
            let output = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^v')",
                ])
                .output()
                .map_err(|e| CommandError::Internal {
                    message: format!("powershell: {e}"),
                })?;
            ensure_command_succeeded("powershell", &output)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[test]
    #[cfg(unix)]
    fn ensure_command_succeeded_accepts_success() {
        let output = std::process::Output {
            status: exit_status(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        assert!(ensure_command_succeeded("osascript", &output).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn ensure_command_succeeded_rejects_failure_with_stderr() {
        let output = std::process::Output {
            status: exit_status(1),
            stdout: Vec::new(),
            stderr: b"AppleEvent timed out".to_vec(),
        };
        let err = ensure_command_succeeded("osascript", &output).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("failed"));
        assert!(msg.contains("AppleEvent timed out"));
    }
}
