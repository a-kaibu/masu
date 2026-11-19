use anyhow::{Context, Result};
use std::{io::Write, path::Path};

/// Publish a complete file with a same-directory rename.
pub(crate) fn publish_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().context("missing store directory")?)?;
    tmp.write_all(bytes)?;
    tmp.persist(path)?;
    Ok(())
}
