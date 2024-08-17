// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

use clap::Parser;


/// A program for sending emails.
#[derive(Debug, Parser)]
#[clap(version = env!("VERSION"))]
pub(crate) struct Args {
  /// The message to send.
  ///
  /// If not specified it will be read from standard input.
  pub message: Option<String>,
  /// The subject to use for the email.
  #[clap(short, long)]
  pub subject: Option<String>,
  /// The content type used for the email; defaults to 'text/plain' if
  /// not provided.
  ///
  /// See https://www.iana.org/assignments/media-types/media-types.xhtml
  #[clap(long)]
  pub content_type: Option<String>,
  /// The path to the configuration file.
  #[clap(short, long)]
  pub config: Option<PathBuf>,
}
