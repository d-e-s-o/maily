// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::future::ready;
use std::process::Output;
use std::process::Stdio;

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use futures::future::Either;
use futures::TryStreamExt as _;

use futures::stream::FuturesUnordered;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::process::Command;


/// Concatenate a command and its arguments into a single string.
fn concat_command<C, A, S>(command: C, args: A) -> OsString
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  args
    .into_iter()
    .fold(command.as_ref().to_os_string(), |mut cmd, arg| {
      cmd.push(OsStr::new(" "));
      cmd.push(arg.as_ref());
      cmd
    })
}


/// Format a command with the given list of arguments as a string.
fn format_command<C, A, S>(command: C, args: A) -> String
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  concat_command(command, args).to_string_lossy().to_string()
}


fn evaluate<C, A, S>(output: &Output, command: C, args: A) -> Result<()>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  if !output.status.success() {
    let code = if let Some(code) = output.status.code() {
      format!(" ({code})")
    } else {
      " (terminated by signal)".to_string()
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim_end();
    let stderr = if !stderr.is_empty() {
      format!(": {stderr}")
    } else {
      String::new()
    };

    bail!(
      "`{}` reported non-zero exit-status{code}{stderr}",
      format_command(command, args),
    );
  }
  Ok(())
}

pub async fn pipeline<I, E, C, A, S>(input: &[u8], commands: I) -> Result<Cow<[u8]>>
where
  I: IntoIterator<IntoIter = E>,
  E: ExactSizeIterator<Item = (C, A)>,
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S> + Clone,
  S: AsRef<OsStr>,
{
  let mut commands = commands.into_iter().peekable();

  if commands.peek().is_some() {
    let mut procs = Vec::with_capacity(commands.len());
    let mut stdout = None;
    let mut stdin = None;

    for (command, args) in commands {
      let mut child = Command::new(command.as_ref())
        .stdin(
          stdout
            .map(|stdout| {
              TryInto::<Stdio>::try_into(stdout)
                .context("failed to convert tokio stdout object into std one")
            })
            .transpose()?
            .unwrap_or_else(Stdio::piped),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(args.clone())
        .spawn()
        .with_context(|| {
          format!(
            "failed to run `{}`",
            format_command(command.as_ref(), args.clone())
          )
        })?;

      if stdin.is_none() {
        stdin = child.stdin.take();
        debug_assert!(stdin.is_some());
      }

      stdout = child.stdout.take();
      debug_assert!(stdout.is_some());

      let () = procs.push((child, command, args));
    }

    let mut output = Vec::new();
    let read_future = async {
      // SANITY: We are guaranteed to have created at least one process
      //         and kept its stdout handle around.
      let mut stdout = stdout.unwrap();
      let _count = stdout
        .read_to_end(&mut output)
        .await
        .context("failed to read data from stdout")?;
      Result::<_, Error>::Ok(())
    };

    // Use a move closure here to make sure that stdin is flushed and
    // closed once we are done writing.
    let write_future = async move {
      // SANITY: We are guaranteed to have created the first process
      //         with an stdin pipe and have the write end available
      //         here.
      let _count = stdin
        .unwrap()
        .write_all(input)
        .await
        .context("failed to write input to stdin")?;
      Result::<_, Error>::Ok(())
    };

    let futures = procs
      .into_iter()
      .map(|(child, command, args)| async {
        let output = child.wait_with_output().await.with_context(|| {
          format!(
            "failed to wait for `{}`",
            format_command(command.as_ref(), args.clone())
          )
        })?;
        let () = evaluate(&output, command, args)?;
        Ok(())
      })
      .map(Either::Left);

    let () = [
      Either::Right(Either::Left(write_future)),
      Either::Right(Either::Right(read_future)),
    ]
    .into_iter()
    .chain(futures)
    .collect::<FuturesUnordered<_>>()
    .try_for_each_concurrent(None, |()| ready(Ok(())))
    .await?;

    Ok(Cow::Owned(output))
  } else {
    Ok(Cow::Borrowed(input))
  }
}


#[cfg(test)]
mod tests {
  use super::*;

  use tokio::test;


  /// Check that we can create a proper pipeline of commands.
  #[test]
  async fn command_chaining() {
    let input = b"foobar";
    #[allow(clippy::useless_conversion)]
    let output = pipeline(input, <[(&OsStr, [&OsStr; 0]); 0]>::from([]))
      .await
      .unwrap();
    assert_eq!(output.as_ref(), input);

    let output = pipeline(input, [("tr", ["o", "e"])]).await.unwrap();
    assert_eq!(output.as_ref(), b"feebar");

    let output = pipeline(input, [("tr", ["o", "e"]), ("tr", ["r", "x"])])
      .await
      .unwrap();
    assert_eq!(output.as_ref(), b"feebax");
  }

  /// Check that we detect errors in a pipeline of commands.
  #[test]
  #[ignore = "relies on some platform dependent behavior"]
  async fn command_chaining_errors() {
    let input = b"foobar";

    let err = pipeline(
      input,
      [
        ("false", [].as_slice()),
        ("tr", ["o", "e"].as_slice()),
        ("tr", ["r", "x"].as_slice()),
      ],
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "`false` reported non-zero exit-status (1)");

    let err = pipeline(
      input,
      [
        ("tr", ["o", "e"].as_slice()),
        ("false", [].as_slice()),
        ("tr", ["r", "x"].as_slice()),
      ],
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "`false` reported non-zero exit-status (1)");

    let err = pipeline(
      input,
      [
        ("tr", ["o", "e"].as_slice()),
        ("tr", ["r", "x"].as_slice()),
        ("false", [].as_slice()),
      ],
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "`false` reported non-zero exit-status (1)");
  }

  /// Check that we can send a lot of data through the pipeline.
  #[test]
  async fn excessive_data_pipeline() {
    let input = (0..8 * 1024 * 1024).map(|_| b'x').collect::<Vec<_>>();
    #[allow(clippy::useless_conversion)]
    let output = pipeline(&input, <[(_, [&OsStr; 0]); 1]>::from([("cat", [])]))
      .await
      .unwrap();
    assert_eq!(output.as_ref(), input);
  }
}
