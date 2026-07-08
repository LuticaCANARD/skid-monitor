/// sshfs client for file sharing
use std::{
    process, thread,time,
    sync::{Arc, Mutex, mpsc}, 
    fs,path
};


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
}

impl<'a> Drop for SshfsClient<'a> {
    fn drop(&mut self) {
        if let Some(mut process) = self.sshfs_process.take() {
            let _ = process.kill();
        }
    }
}

impl<'a> SshfsClient<'a> {
    pub fn new(sshfs_option: SshfsOption<'a>) -> Self {
        Self {
            sshfs_option,
            sshfs_process: None,
        }
    }

    pub fn mount(&mut self) -> Result<(), String> {
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
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn unmount(&mut self) -> Result<(), String> {
        if let Some(mut process) = self.sshfs_process.take() {
            match process.kill() {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("Failed to kill sshfs process: {}", e)),
            }
        } else {
            Err("No sshfs process to unmount".to_string())
        }
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
    let sshfs_command = format!(
        "sshfs -o allow_other,default_permissions,port={} {}@{}:{} {}",
        sshfs_port, sshfs_user, sshfs_host, sshfs_path, mount_path
    );

    match process::Command::new("sh")
        .arg("-c")
        .arg(sshfs_command)
        .spawn()
    {
        Ok(child) => Ok(child),
        Err(e) => Err(format!("Failed to execute sshfs command: {}", e)),
    }
}