// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;

#[cfg(feature = "config")]
use serde::Deserialize;


#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "config", derive(Deserialize))]
#[non_exhaustive]
pub enum SmtpMode {
  /// Use unencrypted SMTP (typically on port 25).
  #[cfg_attr(feature = "config", serde(rename = "unencrypted"))]
  Unencrypted,
  /// Use StartTLS mode (often on port 587).
  #[cfg_attr(feature = "config", serde(rename = "starttls"))]
  StartTls,
  /// Use full TLS mode (often on port 465).
  #[cfg_attr(feature = "config", serde(rename = "tls"))]
  Tls,
}


/// A type representing a single email account.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "config", derive(Deserialize))]
pub struct Account<'input> {
  /// The hostname of the SMTP server.
  pub smtp_host: Cow<'input, str>,
  /// The SMTP "mode" to use.
  pub smtp_mode: SmtpMode,
  /// The "From" identifier to use.
  pub from: Cow<'input, str>,
  /// The user to log in as.
  pub user: Cow<'input, str>,
  /// The password to use for logging in.
  pub password: Cow<'input, str>,
}


#[cfg(feature = "config")]
#[cfg_attr(docsrs, doc(cfg(feature = "config")))]
mod implementation {
  use super::*;

  use std::marker::PhantomData;
  use std::path::Path;
  use std::path::PathBuf;

  use anyhow::Context as _;
  use anyhow::Result;

  use serde_json::from_slice as from_json;

  use tokio::fs::read;

  use crate::EmailOpts;


  /// A type representing a deserializable configuration for the
  /// email sending functionality.
  #[derive(Debug, Deserialize)]
  pub struct Config {
    /// The known accounts.
    pub accounts: Vec<Account<'static>>,
    /// The list of (default) recipients to send each email to.
    pub recipients: Vec<String>,
    /// PGP encrypt any email using the provided keybox file.
    ///
    /// The referenced keybox needs to contain public keys for all
    /// provided recipients.
    #[cfg(feature = "pgp")]
    #[cfg_attr(docsrs, doc(cfg(feature = "pgp")))]
    #[serde(alias = "pgp-keybox")]
    pub pgp_keybox: Option<PathBuf>,
  }

  impl Config {
    /// Destruct this object into constituent part directly usable as
    /// inputs to email sending APIs such as
    /// [`send_email`][crate::send_email].
    ///
    /// # Returns
    /// The function returns a tuple comprised of a list of accounts, a
    /// list of recipients, and an [`EmailOpts`] object.
    pub fn into_inputs(self) -> (Vec<Account<'static>>, Vec<String>, EmailOpts<'static>) {
      let Self {
        accounts,
        recipients,
        #[cfg(feature = "pgp")]
        pgp_keybox,
      } = self;

      let opts = EmailOpts {
        #[cfg(feature = "pgp")]
        pgp_keybox: pgp_keybox.map(Cow::Owned),
        _phantom: PhantomData,
      };

      (accounts, recipients, opts)
    }
  }


  /// Retrieve the path to the system configuration.
  #[inline]
  pub fn system_config_path() -> Result<Cow<'static, Path>> {
    let path = Cow::Borrowed(Path::new("/etc/maily/config.json"));
    Ok(path)
  }


  /// Load the system configuration.
  pub async fn system_config() -> Result<Config> {
    let path = system_config_path().context("failed to retrieve path to system configuration")?;
    let data = read(&path)
      .await
      .with_context(|| format!("failed to read configuration file `{}`", path.display()))?;
    let config = from_json::<Config>(&data)
      .with_context(|| format!("failed to parse `{}` contents as JSON", path.display()))?;
    Ok(config)
  }
}

#[cfg(feature = "config")]
pub use implementation::*;
