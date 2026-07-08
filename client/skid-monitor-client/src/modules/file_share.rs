/// sshfs client for file sharing
use std::{
    process,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SshfsClientState {
    Unmounted,
    Mounted,
    Error(String),
}

pub struct SshfsOption<'a> {
    sshfs_path: &'a str,
    mount_path: &'a str,
    sshfs_user: &'a str,
    sshfs_host: &'a str,
    sshfs_port: u16,
}

pub struct SshfsClient<'a> {
    sshfs_option: SshfsOption<'a>,
    sshfs_process: Option<process::Child>,
    state: Arc<Mutex<SshfsClientState>>,
}

impl<'a> Drop for SshfsClient<'a> {
    fn drop(&mut self) {
        let _ = self.unmount();
    }
}

impl<'a> SshfsClient<'a> {
    pub fn new(sshfs_option: SshfsOption<'a>) -> Self {
        Self {
            sshfs_option,
            sshfs_process: None,
            state: Arc::new(Mutex::new(SshfsClientState::Unmounted)),
        }
    }

    pub fn mount(&mut self) -> Result<(), String> {
        if self.sshfs_process.is_some() {
            return Err("sshfs is already mounted".to_string());
        }

        let SshfsOption {
            sshfs_path,
            mount_path,
            sshfs_user,
            sshfs_host,
            sshfs_port,
        } = &self.sshfs_option;

        match sshfs_open(sshfs_path, mount_path, sshfs_user, sshfs_host, *sshfs_port) {
            Ok(child) => {
                self.sshfs_process = Some(child);
                *self.state.lock().unwrap() = SshfsClientState::Mounted;
                Ok(())
            }
            Err(e) => {
                *self.state.lock().unwrap() = SshfsClientState::Error(e.clone());
                Err(e)
            }
        }
    }

    pub fn unmount(&mut self) -> Result<(), String> {
        let mut process = match self.sshfs_process.take() {
            Some(process) => process,
            None => return Err("No sshfs process to unmount".to_string()),
        };

        // 프로세스만 kill하면 FUSE 마운트가 "Transport endpoint is not
        // connected" 상태로 남을 수 있어 fusermount로 먼저 언마운트한다.
        let _ = process::Command::new("fusermount")
            .arg("-u")
            .arg(self.sshfs_option.mount_path)
            .status();

        let _ = process.kill();
        let _ = process.wait(); // 좀비 프로세스 방지를 위해 반드시 회수

        *self.state.lock().unwrap() = SshfsClientState::Unmounted;
        Ok(())
    }

    pub fn get_state(&self) -> SshfsClientState {
        self.state.lock().unwrap().clone()
    }
}

/// ssh open

pub fn sshfs_open(
    sshfs_path: &str,
    mount_path: &str,
    sshfs_user: &str,
    sshfs_host: &str,
    sshfs_port: u16,
) -> Result<process::Child, String> {
    process::Command::new("sshfs")
        .arg("-o")
        .arg(format!(
            "allow_other,default_permissions,port={}",
            sshfs_port
        ))
        .arg(format!("{}@{}:{}", sshfs_user, sshfs_host, sshfs_path))
        .arg(mount_path)
        .spawn()
        .map_err(|e| format!("Failed to execute sshfs command: {}", e))
}
