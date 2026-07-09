use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

pub const DEFAULT_DASHBOARD_URL: &str = "http://127.0.0.1:8036";

#[derive(Debug, Parser)]
#[command(name = "yc", version, about = "YCloud Console CLI")]
pub struct Cli {
    #[arg(long, global = true, env = "YC_DASHBOARD_URL")]
    pub dashboard_url: Option<String>,

    #[arg(long, global = true, env = "YC_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn config_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.config {
            return Ok(path.clone());
        }
        let home = dirs::home_dir().context("failed to resolve home directory")?;
        Ok(home.join(".yc").join("config.toml"))
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Login(LoginArgs),
    Whoami,
    Contacts {
        #[command(subcommand)]
        command: ContactsCommand,
    },
    Tenants {
        #[command(subcommand)]
        command: TenantsCommand,
    },
    Refresh,
    Logout,
}

#[derive(Debug, Subcommand)]
pub enum TenantsCommand {
    List,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(long, default_value = "developers")]
    pub scope: String,

    #[arg(long)]
    pub code: Option<String>,

    #[arg(long)]
    pub code_verifier: Option<String>,

    #[arg(long)]
    pub state: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ContactsCommand {
    List(ContactsListArgs),
}

#[derive(Debug, Args)]
pub struct ContactsListArgs {
    #[arg(long, default_value_t = 1)]
    pub page_no: u32,

    #[arg(long, default_value_t = 10)]
    pub page_size: u32,

    #[arg(long)]
    pub condition: Option<String>,
}
