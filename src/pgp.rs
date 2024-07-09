use std::fs::File;
use std::io::copy;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use anyhow::Context as _;
use anyhow::Result;

use sequoia_cert_store::store::Certs;
use sequoia_cert_store::store::UserIDQueryParams;
use sequoia_cert_store::Store as _;
use sequoia_cert_store::StoreUpdate as _;
use sequoia_openpgp::armor::Kind;
use sequoia_openpgp::cert::amalgamation::ValidAmalgamation as _;
use sequoia_openpgp::cert::raw::RawCertParser;
use sequoia_openpgp::parse::Parse as _;
use sequoia_openpgp::policy::StandardPolicy;
use sequoia_openpgp::serialize::stream::Armorer;
use sequoia_openpgp::serialize::stream::Encryptor2 as Encryptor;
use sequoia_openpgp::serialize::stream::LiteralWriter;
use sequoia_openpgp::serialize::stream::Message;
use sequoia_openpgp::serialize::stream::Recipient;
use sequoia_openpgp::types::KeyFlags;
use sequoia_openpgp::Cert;

fn parse_keybox(keybox: &Path) -> Result<Certs> {
  let keyring = Certs::empty();
  let f = File::open(keybox)
    .with_context(|| format!("failed to open keyring file `{}`", keybox.display()))?;
  let parser = RawCertParser::from_reader(f)
    .with_context(|| format!("failed to parse keyring `{}`", keybox.display()))?;

  for result in parser {
    let cert =
      result.with_context(|| format!("failed to parse certificate in `{}`", keybox.display()))?;
    let () = keyring
      .update(Arc::new(cert.into()))
      .context("failed to add certificate to store")?;
  }
  Ok(keyring)
}

fn find_recipient_certs<R, S>(keyring: &Certs, recipients: R) -> Result<Vec<Cert>>
where
  R: IntoIterator<Item = S>,
  S: AsRef<str>,
{
  let mut params = UserIDQueryParams::new();
  params.set_ignore_case(true);
  params.set_email(true);

  let mut certs = Vec::new();
  for recipient in recipients {
    let lazy_certs = keyring
      .select_userid(&params, recipient.as_ref())
      .context("failed to find recipient `{recipient}` in keyring")?;

    for lazy_cert in lazy_certs {
      let cert = lazy_cert
        .to_cert()
        .context("failed to parse certificate")?
        .clone();
      let () = certs.push(cert);
    }
  }
  Ok(certs)
}

pub(crate) fn encrypt<R, S>(message: &[u8], keybox: &Path, recipients: R) -> Result<Vec<u8>>
where
  R: IntoIterator<Item = S>,
  S: AsRef<str>,
{
  let mut recipients = recipients.into_iter().peekable();
  if recipients.peek().is_none() {
    return Err(anyhow!("no recipients given"));
  }

  let keyring = parse_keybox(keybox)?;
  let certs = find_recipient_certs(&keyring, recipients)?;

  let mode = KeyFlags::empty().set_transport_encryption();
  let policy = StandardPolicy::default();

  // Build a vector of recipients to hand to Encryptor.
  let mut recipient_subkeys = Vec::<Recipient>::new();
  for cert in certs.iter() {
    let mut count = 0;
    for key in cert
      .keys()
      .with_policy(&policy, None)
      .alive()
      .revoked(false)
      .key_flags(&mode)
      .supported()
      .map(|ka| ka.key())
    {
      recipient_subkeys.push(key.into());
      count += 1;
    }

    if count == 0 {
      let mut expired_keys = Vec::new();
      for ka in cert
        .keys()
        .with_policy(&policy, None)
        .revoked(false)
        .key_flags(&mode)
        .supported()
      {
        let key = ka.key();
        let () = expired_keys.push((
          ka.binding_signature()
            .key_expiration_time(key)
            .context("key does not have an expiration time")?,
          key,
        ));
      }

      let () = expired_keys.sort_by_key(|(expiration_time, _)| *expiration_time);

      if expired_keys.last().is_some() {
        return Err(anyhow!(
          "the last suitable encryption key of cert `{cert}` expired"
        ));
      } else {
        return Err(anyhow!(
          "certificate `{cert}` has no suitable encryption key"
        ));
      }
    }
  }

  let mut buffer = Vec::new();
  let out_msg = Message::new(&mut buffer);
  let armorer = Armorer::new(out_msg)
    .kind(Kind::Message)
    .build()
    .context("failed to create ASCII armorer")?;
  let encryptor = Encryptor::for_recipients(armorer, recipient_subkeys);
  let sink = encryptor.build().context("failed to create encryptor")?;

  let mut literal_writer = LiteralWriter::new(sink)
    .build()
    .context("failed to create literal writer")?;

  // Finally, copy the input message our writer stack to encrypt the
  // data.
  let mut input = message;
  let () = copy(&mut input, &mut literal_writer)
    .map(|_count| ())
    .context("failed to write input to literal writer")?;
  let () = literal_writer.finalize().context("failed to encrypt")?;

  Ok(buffer)
}
