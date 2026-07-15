use crate::{BackendError, BackendErrorCode, EventSink};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct TerminalManager {
    sessions: Arc<Mutex<HashMap<String, TerminalSession>>>,
}

impl TerminalManager {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        events: Arc<dyn EventSink>,
        terminal_id: String,
        worktree_path: String,
        cols: u16,
        rows: u16,
        command: Option<String>,
        command_args: Option<Vec<String>>,
    ) -> Result<(), BackendError> {
        if terminal_id.is_empty() {
            return Err(invalid("terminalId"));
        }
        if self.has(&terminal_id) {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Terminal already exists",
            ));
        }
        let cols = cols.max(1);
        let rows = rows.max(1);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(terminal_error)?;
        let cwd = if Path::new(&worktree_path).is_dir() {
            worktree_path.clone()
        } else {
            std::env::temp_dir().to_string_lossy().into_owned()
        };
        let mut builder = build_command(command.as_deref(), command_args.as_deref())?;
        builder.cwd(cwd);
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        builder.env("JEAN_WORKTREE_PATH", &worktree_path);
        let child = pair.slave.spawn_command(builder).map_err(terminal_error)?;
        let mut reader = pair.master.try_clone_reader().map_err(terminal_error)?;
        let writer = pair.master.take_writer().map_err(terminal_error)?;
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                terminal_id.clone(),
                TerminalSession {
                    master: pair.master,
                    writer: Mutex::new(writer),
                    child,
                },
            );
        events.emit_json(
            "terminal:started",
            serde_json::json!({"terminal_id": terminal_id, "cols": cols, "rows": rows}),
        )?;

        let sessions = self.sessions.clone();
        std::thread::spawn(move || {
            let mut decoder = Utf8StreamDecoder::default();
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        if let Some(data) = decoder.finish() {
                            emit_output(&events, &terminal_id, data);
                        }
                        break;
                    }
                    Ok(read) => {
                        if let Some(data) = decoder.decode(&buffer[..read]) {
                            emit_output(&events, &terminal_id, data);
                        }
                    }
                    Err(error) => {
                        log::debug!("Terminal {terminal_id} read ended: {error}");
                        break;
                    }
                }
            }
            let session = sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&terminal_id);
            if let Some(mut session) = session {
                let exit_code = session
                    .child
                    .wait()
                    .ok()
                    .map(|status| status.exit_code() as i32);
                let _ = events.emit_json(
                    "terminal:stopped",
                    serde_json::json!({"terminal_id": terminal_id, "exit_code": exit_code, "signal": Value::Null}),
                );
            }
        });
        Ok(())
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), BackendError> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let session = sessions
            .get(terminal_id)
            .ok_or_else(|| not_found(terminal_id))?;
        let mut writer = session
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        writer.write_all(data.as_bytes()).map_err(terminal_error)?;
        writer.flush().map_err(terminal_error)
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), BackendError> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let session = sessions
            .get(terminal_id)
            .ok_or_else(|| not_found(terminal_id))?;
        session
            .master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(terminal_error)
    }

    pub fn stop(&self, events: &dyn EventSink, terminal_id: &str) -> Result<bool, BackendError> {
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(terminal_id);
        let Some(mut session) = session else {
            return Ok(false);
        };
        session.child.kill().map_err(terminal_error)?;
        events.emit_json(
            "terminal:stopped",
            serde_json::json!({"terminal_id": terminal_id, "exit_code": Value::Null, "signal": Value::Null}),
        )?;
        Ok(true)
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn has(&self, terminal_id: &str) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(terminal_id)
    }

    pub fn kill_all(&self) -> usize {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let count = sessions.len();
        for (_, mut session) in sessions.drain() {
            let _ = session.child.kill();
        }
        count
    }
}

fn build_command(
    command: Option<&str>,
    command_args: Option<&[String]>,
) -> Result<CommandBuilder, BackendError> {
    let shell = default_shell();
    match command {
        Some("") => Err(BackendError::new(
            BackendErrorCode::InvalidArgument,
            "Command is empty",
        )),
        Some(command) if command_args.is_some() => {
            let mut builder = CommandBuilder::new(command);
            for arg in command_args.unwrap_or_default() {
                builder.arg(arg);
            }
            Ok(builder)
        }
        Some(command) => {
            let mut builder = CommandBuilder::new(shell);
            if cfg!(windows) {
                builder.arg("-Command");
            } else {
                builder.arg("-c");
            }
            builder.arg(command);
            Ok(builder)
        }
        None => Ok(CommandBuilder::new(shell)),
    }
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn emit_output(events: &Arc<dyn EventSink>, terminal_id: &str, data: String) {
    let _ = events.emit_json(
        "terminal:output",
        serde_json::json!({"terminal_id": terminal_id, "data": data}),
    );
}

#[derive(Default)]
struct Utf8StreamDecoder {
    carry: Vec<u8>,
}

impl Utf8StreamDecoder {
    fn decode(&mut self, bytes: &[u8]) -> Option<String> {
        self.carry.extend_from_slice(bytes);
        match std::str::from_utf8(&self.carry) {
            Ok(valid) => {
                let output = valid.to_string();
                self.carry.clear();
                (!output.is_empty()).then_some(output)
            }
            Err(error) if error.error_len().is_none() => {
                let valid =
                    String::from_utf8_lossy(&self.carry[..error.valid_up_to()]).into_owned();
                self.carry.drain(..error.valid_up_to());
                (!valid.is_empty()).then_some(valid)
            }
            Err(_) => {
                let output = String::from_utf8_lossy(&self.carry).into_owned();
                self.carry.clear();
                (!output.is_empty()).then_some(output)
            }
        }
    }

    fn finish(&mut self) -> Option<String> {
        let output = String::from_utf8_lossy(&self.carry).into_owned();
        self.carry.clear();
        (!output.is_empty()).then_some(output)
    }
}

fn terminal_error(error: impl std::fmt::Display) -> BackendError {
    BackendError::new(BackendErrorCode::Io, error.to_string())
}

fn invalid(field: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidArgument,
        format!("Missing or invalid field '{field}'"),
    )
}

fn not_found(terminal_id: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidArgument,
        format!("Terminal not found: {terminal_id}"),
    )
}

pub fn read_run_scripts(worktree_path: &str) -> Vec<String> {
    crate::read_jean_config(worktree_path)
        .and_then(|config| config.scripts.run)
        .map(crate::RunScript::into_vec)
        .unwrap_or_default()
}

pub fn read_ports(worktree_path: &str) -> Vec<Value> {
    crate::read_jean_config(worktree_path)
        .and_then(|config| config.ports)
        .and_then(|ports| serde_json::to_value(ports).ok())
        .and_then(|ports| ports.as_array().cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ServerEventSink, WsBroadcaster};

    #[test]
    fn decoder_carries_split_utf8_codepoint() {
        let mut decoder = Utf8StreamDecoder::default();
        assert_eq!(decoder.decode(&[0xe2, 0x82]), None);
        assert_eq!(decoder.decode(&[0xac]), Some("€".to_string()));
    }

    #[test]
    fn reads_terminal_configuration() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("jean.json"),
            r#"{"scripts":{"run":["bun dev","bun test"]},"ports":[{"port":3000,"label":"web"}]}"#,
        )
        .unwrap();
        assert_eq!(read_run_scripts(temp.path().to_str().unwrap()).len(), 2);
        assert_eq!(read_ports(temp.path().to_str().unwrap())[0]["port"], 3000);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_streams_real_pty_output_without_tauri() {
        let temp = tempfile::tempdir().unwrap();
        let broadcaster = Arc::new(WsBroadcaster::new());
        broadcaster.set_active(true);
        let events: Arc<dyn EventSink> = Arc::new(ServerEventSink::new(broadcaster.clone()));
        let mut receiver = broadcaster.subscribe();
        let manager = TerminalManager::default();
        manager
            .start(
                events,
                "terminal-1".to_string(),
                temp.path().to_string_lossy().into_owned(),
                80,
                24,
                Some("printf 'headless-terminal-ok'".to_string()),
                None,
            )
            .unwrap();
        let output = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = receiver.recv().await.unwrap();
                let value: Value = serde_json::from_str(&event.json).unwrap();
                if value["event"] == "terminal:output" {
                    break value["payload"]["data"].as_str().unwrap().to_string();
                }
            }
        })
        .await
        .unwrap();
        assert!(output.contains("headless-terminal-ok"));
    }
}
