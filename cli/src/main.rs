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

use std::borrow::Cow;
use std::env::args_os;
use std::ffi::OsString;
use std::io;
use std::io::IsTerminal as _;

use clap::Parser as _;

use anyhow::ensure;
use anyhow::Context as _;
use anyhow::Result;

use maily::send_email;
use maily::system_config_path;

use serde_json::from_slice as from_json;

use tokio::fs::read;
use tokio::io::stdin;
use tokio::io::AsyncReadExt as _;

use crate::args::Args;
use crate::config::Config;
use crate::config::Filter;
use crate::util::pipeline;


async fn run_impl(args: Args) -> Result<()> {
  let Args {
    message,
    subject,
    content_type,
    config,
  } = args;

  let path = if let Some(config) = config {
    Cow::Owned(config)
  } else {
    system_config_path()?
  };
  let data = read(&path)
    .await
    .with_context(|| format!("failed to read configuration file `{}`", path.display()))?;
  let config = from_json::<Config>(&data)
    .with_context(|| format!("failed to parse `{}` contents as JSON", path.display()))?;
  let Config { maily, filters } = config;

  ensure!(
    !maily.accounts.is_empty(),
    "no email accounts configured in `{}`",
    path.display()
  );

  let message = if let Some(message) = message {
    message.into_bytes()
  } else {
    // At this point tokio's stdin does not sport the `is_terminal`
    // method so we have to go through std here.
    if io::stdin().is_terminal() {
      println!("Please enter message (terminate with Ctrl-D):");
    }

    let mut data = Vec::new();
    let _count = stdin()
      .read_to_end(&mut data)
      .await
      .context("failed to read message from stdin")?;
    data
  };

  let message = pipeline(&message, filters.into_iter().map(Filter::into))
    .await
    .context("failed to apply filters to message")?;
  let subject = subject.as_deref().unwrap_or("");
  let (accounts, recipients, opts) = maily.into_inputs();

  send_email(
    accounts.iter(),
    subject,
    &message,
    content_type.as_deref(),
    recipients.iter(),
    &opts,
  )
  .await
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
      _ => return Err(err.into()),
    },
  };

  run_impl(args).await
}


#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
  run(args_os()).await
}
