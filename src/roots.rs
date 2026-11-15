//! `mattertui fetch-roots`: download the production attestation roots
//! (PAA + CD signing certs) that retail Matter devices are signed with.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use matter_controller::AttestationTrust;
use serde::Deserialize;

use crate::paths::Paths;

const API: &str =
    "https://api.github.com/repos/project-chip/connectedhomeip/contents/credentials/production";
const USER_AGENT: &str = "mattertui";

#[derive(Deserialize)]
struct Entry {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    download_url: Option<String>,
}

/// Download into a staging dir, validate, then swap into place so a failed
/// run never leaves a half-written trust store.
pub fn fetch(paths: &Paths) -> Result<()> {
    paths.ensure_data_dir()?;
    let roots = paths.roots_dir();
    let staging = paths.data_dir.join("roots.new");
    let _ = fs::remove_dir_all(&staging);

    let paa = staging.join("paa");
    let cd = staging.join("cd");
    let n_paa = download_dir("paa-root-certs", &paa)?;
    let n_cd = download_dir("cd-certs", &cd)?;

    let dropped = validate(&paa, &cd)?;
    for name in &dropped {
        eprintln!("  skipped unparseable certificate {name}");
    }

    let old = paths.data_dir.join("roots.old");
    let _ = fs::remove_dir_all(&old);
    if roots.exists() {
        fs::rename(&roots, &old).context("moving old roots aside")?;
    }
    fs::rename(&staging, &roots).context("installing new roots")?;
    let _ = fs::remove_dir_all(&old);

    eprintln!(
        "installed {} PAA and {n_cd} CD certificates into {}",
        n_paa - dropped.len(),
        roots.display()
    );
    Ok(())
}

fn download_dir(remote: &str, local: &Path) -> Result<usize> {
    fs::create_dir_all(local)?;
    let listing: Vec<Entry> = serde_json::from_str(
        &get(&format!("{API}/{remote}"))?
            .read_to_string()
            .with_context(|| format!("reading {remote} listing"))?,
    )
    .with_context(|| format!("parsing {remote} listing"))?;

    let ders: Vec<&Entry> = listing
        .iter()
        .filter(|e| e.kind == "file" && e.name.ends_with(".der"))
        .collect();
    if ders.is_empty() {
        bail!("no .der files found in {remote}");
    }
    eprintln!("downloading {} certificates from {remote}…", ders.len());
    for e in &ders {
        // Names come from the API; refuse anything that isn't a plain file name.
        if e.name.contains('/') || e.name.starts_with('.') {
            bail!("unexpected file name in listing: {}", e.name);
        }
        let url = e.download_url.as_deref().context("entry has no download_url")?;
        let bytes = get(url)?
            .read_to_vec()
            .with_context(|| format!("downloading {}", e.name))?;
        fs::write(local.join(&e.name), bytes)?;
    }
    Ok(ders.len())
}

/// `AttestationTrust::from_dirs` rejects the whole set if any one cert fails
/// to parse. Drop offenders (named in the error) until the set loads.
fn validate(paa: &Path, cd: &Path) -> Result<Vec<String>> {
    let mut dropped = Vec::new();
    loop {
        match AttestationTrust::from_dirs(paa, cd) {
            Ok(_) => return Ok(dropped),
            Err(err) => {
                let msg = err.to_string();
                let culprit = fs::read_dir(paa)?
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .find(|name| msg.contains(name.as_str()));
                match culprit {
                    Some(name) => {
                        fs::remove_file(paa.join(&name))?;
                        dropped.push(name);
                    }
                    None => return Err(err).context("downloaded roots do not load"),
                }
            }
        }
    }
}

fn get(url: &str) -> Result<ureq::Body> {
    let resp = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp.into_body())
}
