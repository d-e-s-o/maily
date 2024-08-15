// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(
  clippy::collapsible_if,
  clippy::fn_to_numeric_cast,
  clippy::let_and_return,
  clippy::let_unit_value
)]

mod config;
mod rand;

use std::str::FromStr as _;

use anyhow::anyhow;
use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use lettre::message::header::ContentTransferEncoding;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::AsyncSmtpTransport;
use lettre::AsyncTransport;
use lettre::Message;
use lettre::Tokio1Executor;

pub use crate::config::Account;
pub use crate::config::SmtpMode;

use crate::rand::RandExt as _;
use crate::rand::Rng;


async fn try_send_email<R, S>(
  account: &Account<'_>,
  subject: &str,
  message: &[u8],
  recipients: R,
  transfer_encoding: Option<&str>,
) -> Result<()>
where
  R: Iterator<Item = S> + Clone,
  S: AsRef<str>,
{
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
    let recipient = recipient.as_ref();
    let to = recipient
      .parse()
      .with_context(|| format!("failed to parse 'To' specification: `{recipient}`"))?;

    email = email.to(to);
  }

  let email = email
    .body(message.to_vec())
    .context("failed to create email message")?;

  let creds = Credentials::new(account.user.to_string(), account.password.to_string());

  let mailer = match account.smtp_mode {
    SmtpMode::Unencrypted => {
      AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(account.smtp_host)
        .credentials(creds)
        .build()
    },
    SmtpMode::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(account.smtp_host)
      .context("failed to create TLS SMTP mailer")?
      .credentials(creds)
      .build(),
    SmtpMode::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(account.smtp_host)
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

pub async fn send_email<'acc, A, R, I, S>(
  accounts: A,
  subject: &str,
  message: &[u8],
  recipients: R,
  transfer_encoding: Option<&str>,
) -> Result<()>
where
  A: IntoIterator<Item = &'acc Account<'acc>>,
  R: IntoIterator<IntoIter = I>,
  I: Iterator<Item = S> + Clone,
  S: AsRef<str>,
{
  let mut accounts = accounts.into_iter().collect::<Vec<&Account<'_>>>();
  let rng = Rng::new();
  let () = rng.shuffle(&mut accounts);

  let recipients = recipients.into_iter();

  let mut overall_result = Result::<_, Error>::Ok(());
  for account in accounts {
    if let Err(err) = &overall_result {
      // There isn't really anything that we could do about potential
      // errors here, so just ignore them.
      let _result = try_send_email(
        account,
        "email error",
        format!("{err:?}").as_bytes(),
        recipients.clone(),
        transfer_encoding,
      )
      .await;
    }

    let result = try_send_email(
      account,
      subject,
      message,
      recipients.clone(),
      transfer_encoding,
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
