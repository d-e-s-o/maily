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
mod rand;
mod util;

use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr as _;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use clap::Parser as _;

use dirs::config_dir;

use lettre::message::header::ContentTransferEncoding;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::AsyncSmtpTransport;
use lettre::AsyncTransport;
use lettre::Message;
use lettre::Tokio1Executor;

use serde_json::from_slice as from_json;

use tokio::fs::read;
use tokio::io::stdin;
use tokio::io::AsyncReadExt as _;

use crate::args::Args;
use crate::config::Account;
use crate::config::Config;
use crate::config::Filter;
use crate::config::SmtpMode;
use crate::rand::RandExt as _;
use crate::rand::Rng;
use crate::util::pipeline;


/// Retrieve the path to the program's configuration.
fn config_path() -> Result<PathBuf> {
  let path = config_dir()
    .ok_or_else(|| anyhow!("unable to determine config directory"))?
    .join("mail-message")
    .join("config.json");
  Ok(path)
}


async fn send_email(
  account: &Account,
  subject: &str,
  message: &[u8],
  transfer_encoding: Option<&str>,
  recipients: &[String],
) -> Result<()> {
  let from = account
    .from
    .parse()
    .with_context(|| format!("failed to parse 'From' specification: `{}`", account.from))?;
  let email = Message::builder()
    .from(from)
    .subject(subject)
    .header(ContentType::TEXT_PLAIN);

  // We only set the transfer encoding if the user provided one. The
  // reason being that:
  // > The `Message` builder takes care of choosing the most
  // > efficient encoding based on the chosen body, so in most
  // > use-caches this header shouldn't be set manually.
  let mut email = if let Some(transfer_encoding) = transfer_encoding {
    let transfer_encoding = ContentTransferEncoding::from_str(transfer_encoding)
      .map_err(|_| anyhow!("failed to parse transfer encoding `{transfer_encoding}`"))?;
    email.header(transfer_encoding)
  } else {
    email
  };

  for recipient in recipients {
    let to = recipient
      .parse()
      .with_context(|| format!("failed to parse 'To' specification: `{recipient}`"))?;

    email = email.to(to);
  }

  // Emails typically use \r\n as line ending. Some providers refuse
  // incoming messages not adhering to it and some don't do the
  // conversion for outgoing mails. It's best to do it ourselves.
  let message = String::from_utf8_lossy(message).replace('\n', "\r\n");
  let email = email
    .body(message)
    .context("failed to create email message")?;

  let creds = Credentials::new(account.user.to_string(), account.password.to_string());

  let mailer = match account.smtp_mode {
    SmtpMode::Unencrypted => {
      AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&account.smtp_host)
        .credentials(creds)
        .build()
    },
    SmtpMode::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&account.smtp_host)
      .context("failed to create TLS SMTP mailer")?
      .credentials(creds)
      .build(),
    SmtpMode::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&account.smtp_host)
      .context("failed to create STARTTLS SMTP mailer")?
      .credentials(creds)
      .build(),
  };

  let _mailer = mailer
    .send(email)
    .await
    .with_context(|| format!("failed to send email via {}", account.smtp_host))?;
  Ok(())
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
    mut accounts,
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

  let message = pipeline(&message, filters.into_iter().map(Filter::into))
    .await
    .context("failed to apply filters to message")?;

  let rng = Rng::new();
  let () = rng.shuffle(&mut accounts);

  let mut overall_result = Result::<_, Error>::Ok(());
  for account in accounts {
    if let Err(err) = &overall_result {
      let _result = send_email(
        &account,
        "intermittent email error",
        format!("{err:?}").as_bytes(),
        transfer_encoding.as_deref(),
        &recipients,
      )
      .await;
      // There isn't really anything that we can do about this error.
    }

    let result = send_email(
      &account,
      subject.as_deref().unwrap_or(""),
      &message,
      transfer_encoding.as_deref(),
      &recipients,
    )
    .await;
    match result {
      Ok(()) => return Ok(()),
      Err(err) => {
        if let Err(overall_err) = overall_result {
          overall_result = Err(overall_err.context(err));
        } else {
          overall_result = Err(err);
        }
      },
    }
  }

  overall_result
}


/// Run the program and report errors, if any.
pub async fn run<A, T>(args: A) -> Result<()>
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
