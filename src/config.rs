// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;

use serde::de::Error;
use serde::de::Unexpected;
use serde::Deserialize;
use serde::Deserializer;


/// The program's configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct Config {
  /// The known accounts.
  pub accounts: Vec<Account>,
  /// The list of (default) recipients to send each email to.
  pub recipients: Vec<String>,
  /// The filters to use when sending an email.
  pub filters: Vec<Filter>,
  /// The transfer encoding used for the message.
  #[serde(alias = "transfer-encoding")]
  pub transfer_encoding: Option<String>,
}


#[derive(Debug)]
pub(crate) enum SmtpMode {
  /// Use unencrypted SMTP (typically on port 25).
  Unencrypted,
  /// Use StartTLS mode (often on port 587).
  StartTls,
  /// Use full TLS mode (often on port 465).
  Tls,
}

/// Deserialize a `Mode` from a string.
fn deserialize_smtp_mode<'de, D>(deserializer: D) -> Result<SmtpMode, D::Error>
where
  D: Deserializer<'de>,
{
  let string = Cow::<str>::deserialize(deserializer)?;
  match string.as_ref() {
    "unencrypted" => Ok(SmtpMode::Unencrypted),
    "starttls" => Ok(SmtpMode::StartTls),
    "tls" => Ok(SmtpMode::Tls),
    _ => Err(Error::invalid_value(
      Unexpected::Str(&string),
      &"a valid SMTP mode (one of `unencrypted`, `starttls`, `tls`)",
    )),
  }
}


/// A type representing a single email account.
#[derive(Debug, Deserialize)]
pub(crate) struct Account {
  /// The hostname of the SMTP server.
  pub smtp_host: String,
  /// The SMTP "mode" to use.
  #[serde(deserialize_with = "deserialize_smtp_mode")]
  pub smtp_mode: SmtpMode,
  /// The "From" identifier to use.
  pub from: String,
  /// The user to log in as.
  pub user: String,
  /// The password to use for logging in.
  pub password: String,
}


/// A "filter" for an email.
#[derive(Debug, Deserialize)]
pub(crate) struct Filter {
  /// The command to use for filtering emails.
  pub command: String,
  /// The argument to use.
  pub args: Vec<String>,
}

impl From<Filter> for (String, Vec<String>) {
  fn from(filter: Filter) -> Self {
    (filter.command, filter.args)
  }
}
