// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(
  clippy::collapsible_else_if,
  clippy::collapsible_if,
  clippy::fn_to_numeric_cast,
  clippy::let_and_return,
  clippy::let_unit_value
)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! This library provides infrastructure for easy and quick sending of
//! emails. It comes with built-in redundancy by allowing the
//! configuration of multiple SMTP accounts, one of which will be chosen
//! at random. If sending fails, another one if picked and the operation
//! retried, until one succeeded or all failed sending.
//!
//! If the `pgp` feature is enabled, emails can be PGP encrypted to the
//! given set of recipients.
//!
//! With the `config` feature enable, the library honors a global
//! system-wide configuration. This configuration can capture anything
//! from SMTP account to default recipients and means that clients of
//! this crate don't *have to* specify anything but message contents.

mod config;
#[cfg(feature = "pgp")]
mod pgp;
mod rand;

#[cfg(feature = "pgp")]
use std::borrow::Cow;
use std::marker::PhantomData;
use std::path::Path;
use std::str;

use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use lettre::message::header::ContentDisposition;
use lettre::message::header::ContentType;
use lettre::message::MaybeString;
use lettre::message::MultiPart;
use lettre::message::SinglePart;
use lettre::transport::smtp::authentication::Credentials;
use lettre::AsyncSmtpTransport;
use lettre::AsyncTransport;
use lettre::Message;
use lettre::Tokio1Executor;

#[cfg(feature = "config")]
#[cfg_attr(docsrs, doc(cfg(feature = "config")))]
pub use crate::config::system_config;
#[cfg(feature = "config")]
#[cfg_attr(docsrs, doc(cfg(feature = "config")))]
pub use crate::config::system_config_path;
pub use crate::config::Account;
#[cfg(feature = "config")]
#[cfg_attr(docsrs, doc(cfg(feature = "config")))]
pub use crate::config::Config;
pub use crate::config::SmtpMode;

#[cfg(feature = "pgp")]
use crate::pgp::encrypt;
use crate::rand::RandExt as _;
use crate::rand::Rng;


#[cfg(feature = "tracing")]
#[macro_use]
#[allow(unused_imports)]
mod log {
  pub(crate) use tracing::debug;
  pub(crate) use tracing::error;
  pub(crate) use tracing::info;
  pub(crate) use tracing::instrument;
  pub(crate) use tracing::trace;
  pub(crate) use tracing::warn;
}

#[cfg(not(feature = "tracing"))]
#[macro_use]
#[allow(unused_imports)]
mod log {
  macro_rules! debug {
    ($($args:tt)*) => {};
  }
  pub(crate) use debug;
  pub(crate) use debug as error;
  pub(crate) use debug as info;
  pub(crate) use debug as trace;
  pub(crate) use debug as warn;
}


/// A type capturing options for capturing a screenshot.
#[derive(Clone, Debug, Default)]
pub struct EmailOpts<'input> {
  /// PGP encrypt the email using the provided keybox file.
  ///
  /// The referenced keybox needs to contain public keys for all
  /// provided recipients or a runtime error will be reported.
  ///
  /// With GnuPG, a keybox can be created via `gpg --armor --export
  /// 'deso@posteo.net' > keybox.gpg`, for example.
  #[cfg(feature = "pgp")]
  #[cfg_attr(docsrs, doc(cfg(feature = "pgp")))]
  pub pgp_keybox: Option<Cow<'input, Path>>,
  /// The type is non-exhaustive and open to extension.
  #[doc(hidden)]
  pub _phantom: PhantomData<&'input ()>,
}


#[cfg(not(feature = "pgp"))]
fn encrypt<R, S>(_message: &[u8], _keybox: &Path, _recipients: R) -> Result<Vec<u8>>
where
  R: IntoIterator<Item = S>,
  S: AsRef<str>,
{
  unreachable!()
}


#[cfg_attr(feature = "tracing", log::instrument(skip_all, err, fields(subject = subject, from = %account.from)))]
async fn try_send_email<R, S>(
  account: &Account<'_>,
  subject: &str,
  message: &[u8],
  content_type: Option<&str>,
  recipients: R,
  opts: &EmailOpts<'_>,
) -> Result<()>
where
  R: Iterator<Item = S> + Clone,
  S: AsRef<str>,
{
  let from = account
    .from
    .parse()
    .with_context(|| format!("failed to parse 'From' specification: `{}`", account.from))?;
  let content_type = content_type
    .map(|content_type| {
      ContentType::parse(content_type)
        .with_context(|| format!("failed to parse content type specification `{content_type}`"))
    })
    .transpose()?
    .unwrap_or(ContentType::TEXT_PLAIN);
  let mut email = Message::builder().from(from).subject(subject);

  let EmailOpts {
    #[cfg(feature = "pgp")]
    pgp_keybox,
    _phantom: PhantomData,
  } = opts;

  #[cfg(not(feature = "pgp"))]
  let pgp_keybox = None;

  for recipient in recipients.clone() {
    let recipient = recipient.as_ref();
    let to = recipient
      .parse()
      .with_context(|| format!("failed to parse 'To' specification: `{recipient}`"))?;

    email = email.to(to);
  }

  let creds = Credentials::new(account.user.to_string(), account.password.to_string());

  let mailer = match account.smtp_mode {
    SmtpMode::Unencrypted => {
      AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(account.smtp_host.to_string())
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

  let email = if let Some(keybox) = pgp_keybox {
    let inner = MultiPart::mixed().singlepart(
      SinglePart::builder()
        .header(content_type)
        .body(message.to_vec()),
    );

    // TODO: Ideally we'd also sign the message, but that's a different
    //       pandora's box and not as important at this point.
    let message =
      encrypt(&inner.formatted(), keybox, recipients).context("failed to encrypt message")?;
    // We always ASCII armor the message, so we do not expect it to ever
    // be *not* a valid UTF-8 string.
    let message =
      str::from_utf8(&message).context("PGP encrypted message is not a valid UTF-8 string")?;

    let parts = MultiPart::encrypted("application/pgp-encrypted".to_owned())
      .singlepart(
        SinglePart::builder()
          .header(
            ContentType::parse("application/pgp-encrypted")
              .context("failed to parse 'application/pgp-encrypted' content type header")?,
          )
          .body(String::from("Version: 1")),
      )
      .singlepart(
        SinglePart::builder()
          .header(
            ContentType::parse(r#"application/octet-stream; name="encrypted.asc""#)
              .context("failed to parse 'application/octet-stream' content type header")?,
          )
          .header(ContentDisposition::inline_with_name("encrypted.asc"))
          .body(message.to_string()),
      );

    email
      .multipart(parts)
      .context("failed to create email message")?
  } else {
    // We always try to work with string. The reason being that `lettre`
    // performs line ending conversion only when the data is passed in
    // as a string, and some mailers reject emails with bare linefeed
    // line endings.
    let body = if let Ok(message) = str::from_utf8(message) {
      MaybeString::String(message.to_string())
    } else {
      MaybeString::Binary(message.to_vec())
    };

    email
      .header(content_type)
      .body(body)
      .context("failed to create email message")?
  };

  log::trace!(email = %String::from_utf8_lossy(&email.formatted()));

  let _mailer = mailer
    .send(email)
    .await
    .with_context(|| format!("failed to send email via {}", account.smtp_host))?;

  log::debug!("email sent successfully");
  Ok(())
}


/// Send an email using the provided inputs.
///
/// This function attempts to send an email using one of the accounts
/// provided. Accounts are chosen at random and on send failure another
/// one is tried until one succeeded or all failed sending. In addition,
/// in case of a send failure attempts are made to inform recipients
/// about that via an additional email outlining the error encountered
/// with a different account.
pub async fn send_email<'acc, A, R, I, S>(
  accounts: A,
  subject: &str,
  message: &[u8],
  content_type: Option<&str>,
  recipients: R,
  opts: &EmailOpts<'_>,
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
        None,
        recipients.clone(),
        opts,
      )
      .await;
    }

    let result = try_send_email(
      account,
      subject,
      message,
      content_type,
      recipients.clone(),
      opts,
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
