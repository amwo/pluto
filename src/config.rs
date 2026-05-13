use std::str::FromStr;

use anyhow::{Context, Result};

use crate::adapters::{grpc, http, telegram};
use crate::domain::{Mode, Pubkey};

#[derive(Clone, Debug)]
pub struct Config {
    pub grpc_endpoint: String,
    pub rpc_endpoint: String,
    pub username: String,
    pub password: String,
    pub bot_wallet: Pubkey,
    pub target_wallet: Pubkey,
    pub mode: Mode,
    pub database_url: String,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bot_wallet = std::env::var("SOLANA_WALLET_ADDRESS").context("SOLANA_WALLET_ADDRESS")?;
        let target_wallet = std::env::var("TARGET_WALLET").context("TARGET_WALLET")?;
        let mode = std::env::var("PLUTO_MODE").unwrap_or_else(|_| "paper".to_string());
        Ok(Self {
            grpc_endpoint: std::env::var("CHAINSTACK_GRPC_ENDPOINT")
                .context("CHAINSTACK_GRPC_ENDPOINT")?,
            rpc_endpoint: std::env::var("CHAINSTACK_HTTPS_ENDPOINT")
                .context("CHAINSTACK_HTTPS_ENDPOINT")?,
            username: std::env::var("CHAINSTACK_USERNAME").context("CHAINSTACK_USERNAME")?,
            password: std::env::var("CHAINSTACK_PASSWORD").context("CHAINSTACK_PASSWORD")?,
            bot_wallet: Pubkey::from_base58(&bot_wallet)?,
            target_wallet: Pubkey::from_base58(&target_wallet)?,
            mode: Mode::from_str(&mode).context("PLUTO_MODE")?,
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL")?,
            telegram_bot_token: std::env::var("TELEGRAM_BOT_TOKEN").ok(),
            telegram_chat_id: std::env::var("TELEGRAM_CHAT_ID").ok(),
        })
    }

    pub fn grpc(&self) -> grpc::Endpoint {
        grpc::Endpoint {
            url: self.grpc_endpoint.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }

    pub fn http(&self) -> http::Endpoint {
        http::Endpoint {
            url: self.rpc_endpoint.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }

    pub fn telegram(&self) -> Option<telegram::Endpoint> {
        match (&self.telegram_bot_token, &self.telegram_chat_id) {
            (Some(token), Some(chat_id)) => Some(telegram::Endpoint {
                token: token.clone(),
                chat_id: chat_id.clone(),
            }),
            _ => None,
        }
    }
}
