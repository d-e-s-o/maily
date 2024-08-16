// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(
  clippy::collapsible_if,
  clippy::fn_to_numeric_cast,
  clippy::let_and_return,
  clippy::let_unit_value
)]

mod args;
mod config;
mod util;

use std::env::args_os;
use std::ffi::OsString;
use std::path::PathBuf;

use clap::Parser as _;

use dirs::config_dir;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Context as _;
use anyhow::Result;

use maily::send_email;
use maily::Account;
use maily::EmailOpts;

use serde_json::from_slice as from_json;

use tokio::fs::read;
use tokio::io::stdin;
use tokio::io::AsyncReadExt as _;

use crate::args::Args;
use crate::config::Config;
use crate::config::Filter;
use crate::util::pipeline;


/// Retrieve the path to the program's configuration.
fn config_path() -> Result<PathBuf> {
  let path = config_dir()
    .ok_or_else(|| anyhow!("unable to determine config directory"))?
    .join("mail-message")
    .join("config.json");
  Ok(path)
}


async fn run_impl(args: Args) -> Result<()> {
  let Args {
    message,
    subject,
    config,
  } = args;

  let path = if let Some(config) = config {
    config
  } else {
    config_path()?
  };
  let data = read(&path)
    .await
    .with_context(|| format!("failed to read configuration file `{}`", path.display()))?;
  let config = from_json::<Config>(&data)
    .with_context(|| format!("failed to parse `{}` contents as JSON", path.display()))?;
  let Config {
    accounts,
    recipients,
    filters,
    transfer_encoding,
  } = config;

  ensure!(
    !accounts.is_empty(),
    "no email accounts configured in `{}`",
    path.display()
  );

  let message = if let Some(message) = message {
    message.into_bytes()
  } else {
    println!("Please enter message (terminate with Ctrl-D):");

    let mut data = Vec::new();
    let _count = stdin()
      .read_to_end(&mut data)
      .await
      .context("failed to read message from stdin")?;
    data
  };

  let accounts = accounts.iter().map(Account::from).collect::<Vec<_>>();
  let message = pipeline(&message, filters.into_iter().map(Filter::into))
    .await
    .context("failed to apply filters to message")?;
  let subject = subject.as_deref().unwrap_or("");
  let opts = EmailOpts {
    transfer_encoding: transfer_encoding.as_deref(),
    ..Default::default()
  };

  send_email(accounts.iter(), subject, &message, recipients.iter(), &opts).await
}


/// Run the program and report errors, if any.
async fn run<A, T>(args: A) -> Result<()>
where
  A: IntoIterator<Item = T>,
  T: Into<OsString> + Clone,
{
  let args = match Args::try_parse_from(args) {
    Ok(args) => args,
    Err(err) => match err.kind() {
      clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
        print!("{}", err);
        return Ok(())
      },
      _ => return Err(err).context("failed to parse program arguments"),
    },
  };

  run_impl(args).await
}


#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
  run(args_os()).await
}
