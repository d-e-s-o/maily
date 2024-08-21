// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;


#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum SmtpMode {
  /// Use unencrypted SMTP (typically on port 25).
  Unencrypted,
  /// Use StartTLS mode (often on port 587).
  StartTls,
  /// Use full TLS mode (often on port 465).
  Tls,
}


/// A type representing a single email account.
#[derive(Clone, Debug)]
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
