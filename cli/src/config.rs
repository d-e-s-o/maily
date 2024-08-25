// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use serde::Deserialize;


/// The program's configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct Config {
  #[serde(flatten)]
  pub maily: maily::Config,
  /// The filters to use when sending an email.
  #[serde(default)]
  pub filters: Vec<Filter>,
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
