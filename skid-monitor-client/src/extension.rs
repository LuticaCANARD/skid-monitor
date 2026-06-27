//! Out-of-process extension bridge.
//!
//! The Rust client stays responsible for protocol reception and local rendering.
//! Optional .NET/C# extensions receive newline-delimited JSON events over stdin.

use skid_protocol::protocol::Signal;
use std::io::{self, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

const EXTENSION_HOST_ENV: &str = "SKID_MONITOR_EXTENSION_HOST";

pub struct ExtensionHost {
    _child: Child,
    stdin: ChildStdin,
}

impl ExtensionHost {
    pub fn from_env() -> io::Result<Option<Self>> {
        let command_line = match std::env::var(EXTENSION_HOST_ENV) {
            Ok(value) if !value.trim().is_empty() => value,
            _ => return Ok(None),
        };

        let mut parts = command_line.split_whitespace();
        let program = parts.next().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{EXTENSION_HOST_ENV} does not contain a program"),
            )
        })?;

        let mut child = Command::new(program)
            .args(parts)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "extension host stdin was not available",
            )
        })?;

        eprintln!("skid-monitor extension host started: {command_line}");
        Ok(Some(Self {
            _child: child,
            stdin,
        }))
    }

    pub fn publish_signal(&mut self, signal: &Signal) -> io::Result<()> {
        let event = serde_json::json!({
            "schema": "skid.monitor.extension.v1",
            "type": "signal",
            "signal": signal,
        });
        serde_json::to_writer(&mut self.stdin, &event)?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()
    }
}
