//! The two network operations, built from the pure functions in `release` and `archive`.

use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::apply::Staged;
use crate::archive::extract;
use crate::error::UpdateError;
use crate::install::Install;
use crate::release::{Release, Version, asset_name, checksum_for, newer_release, release_dir_name};
use crate::{CHECKSUMS_FILE, RELEASES_URL, STAGING_PREFIX, UPDATE_CHECK_TIMEOUT, USER_AGENT};

/// Asks GitHub which release is the latest. `Some` only when it is newer than `current`.
///
/// A HEAD request that follows redirects: the old repository name redirects to the new one, and
/// the latest-release page redirects to the tag page, whose URL names the version. The tag page
/// itself is never downloaded.
pub async fn check(current: Version) -> Result<Option<Release>, UpdateError> {
    let response = client()?
        .head(format!("{RELEASES_URL}/latest"))
        .send()
        .await?
        .error_for_status()?;
    newer_release(current, response.url().as_str())
}

/// Downloads the platform archive for `release` into a staging directory inside `install.dir`,
/// checks it against the release's `SHA256SUMS`, and extracts it there. `progress` is called
/// with the bytes received so far and the total when the server states one.
pub async fn download(
    release: &Release,
    install: &Install,
    mut progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<Staged, UpdateError> {
    let staging = install
        .dir
        .join(format!("{STAGING_PREFIX}{}", release.version));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging).map_err(|source| UpdateError::NotWritable {
        dir: install.dir.clone(),
        source,
    })?;
    let outcome = fetch_into(release, &staging, &mut progress).await;
    if outcome.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    outcome
}

async fn fetch_into(
    release: &Release,
    staging: &Path,
    progress: &mut (impl FnMut(u64, Option<u64>) + Send),
) -> Result<Staged, UpdateError> {
    let client = client()?;
    let base = format!("{RELEASES_URL}/download/{}", release.tag);
    let sums = client
        .get(format!("{base}/{CHECKSUMS_FILE}"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let asset = asset_name(release.version);
    let expected = checksum_for(&sums, &asset)?;

    let archive = staging.join(&asset);
    let mut response = client
        .get(format!("{base}/{asset}"))
        .send()
        .await?
        .error_for_status()?;
    let total = response.content_length();
    let mut file = tokio::fs::File::create(&archive).await?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while let Some(chunk) = response.chunk().await? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        progress(received, total);
    }
    file.flush().await?;
    drop(file);
    if hex(&hasher.finalize()) != expected {
        return Err(UpdateError::Checksum(format!(
            "{asset} does not match {CHECKSUMS_FILE}"
        )));
    }

    let top_level = release_dir_name(release.version);
    let files = tokio::task::spawn_blocking({
        let archive = archive.clone();
        let staging = staging.to_path_buf();
        move || extract(&archive, &top_level, &staging)
    })
    .await
    .map_err(|join| UpdateError::Io(io::Error::other(join)))??;
    fs::remove_file(&archive)?;
    Ok(Staged {
        dir: staging.to_path_buf(),
        files,
    })
}

/// One client for both operations: redirects followed (assets live on GitHub's object store),
/// a bounded connect and per-read wait so a dead network fails rather than hangs.
///
/// `reqwest`'s `rustls-no-provider` feature leaves the process-wide crypto provider unset so an
/// app with its own choice (iroh installs `ring` for its own QUIC handshakes) is never overridden
/// silently. `install_default` succeeds at most once per process; a later call from a second
/// `client()` invocation just returns the already-installed provider, which is fine here.
fn client() -> Result<reqwest::Client, UpdateError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(UPDATE_CHECK_TIMEOUT)
        .read_timeout(UPDATE_CHECK_TIMEOUT)
        .build()?)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `client()` installs ring itself, so building it never depends on some other crate having
    /// set the process-level provider first. This proves it: rustls panics when a `Client` is
    /// built with none, and the ordering of that install is easy to lose in a refactor.
    #[test]
    fn the_http_client_builds_with_the_tls_stack_in_the_tree() {
        client().unwrap();
    }

    #[test]
    fn digests_print_as_lowercase_hex() {
        let digest = Sha256::digest(b"");
        assert_eq!(
            hex(&digest),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
