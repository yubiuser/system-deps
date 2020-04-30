//! metadeps lets you write `pkg-config` dependencies in `Cargo.toml` metadata,
//! rather than programmatically in `build.rs`.  This makes those dependencies
//! declarative, so other tools can read them as well.
//!
//! metadeps parses metadata like this in `Cargo.toml`:
//!
//! ```toml
//! [package.metadata.pkg-config]
//! testlib = "1.2"
//! testdata = { version = "4.5", feature = "some-feature" }
//! glib = { name = "glib-2.0", version = "2.64" }
//! gstreamer = { name = "gstreamer-1.0", version = "1.0", feature-versions = { v1_2 = "1.2", v1_4 = "1.4" }}
//! ```

#![deny(missing_docs, warnings)]

#[macro_use]
extern crate error_chain;

use pkg_config::{Config, Library};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use version_compare::VersionCompare;

error_chain! {
    foreign_links {
        PkgConfig(pkg_config::Error) #[doc="pkg-config error"];
    }
}

fn has_feature(feature: &str) -> bool {
    let var = format!("CARGO_FEATURE_{}", feature.to_uppercase().replace('-', "_"));
    env::var_os(var).is_some()
}

/// Probe all libraries configured in the Cargo.toml
/// `[package.metadata.pkg-config]` section.
pub fn probe() -> Result<HashMap<String, Library>> {
    let dir = env::var_os("CARGO_MANIFEST_DIR").ok_or("$CARGO_MANIFEST_DIR not set")?;
    let mut path = PathBuf::from(dir);
    path.push("Cargo.toml");
    let mut manifest =
        fs::File::open(&path).chain_err(|| format!("Error opening {}", path.display()))?;
    let mut manifest_str = String::new();
    manifest
        .read_to_string(&mut manifest_str)
        .chain_err(|| format!("Error reading {}", path.display()))?;
    let toml = manifest_str
        .parse::<toml::Value>()
        .map_err(|e| format!("Error parsing TOML from {}: {:?}", path.display(), e))?;
    let key = "package.metadata.pkg-config";
    let meta = toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("pkg-config"))
        .ok_or(format!("No {} in {}", key, path.display()))?;
    let table = meta
        .as_table()
        .ok_or(format!("{} not a table in {}", key, path.display()))?;
    let mut libraries = HashMap::new();
    for (name, value) in table {
        let (lib_name, version) = match value {
            toml::Value::String(ref s) => (name, s),
            toml::Value::Table(ref t) => {
                let mut feature = None;
                let mut version = None;
                let mut lib_name = None;
                let mut enabled_feature_versions = Vec::new();
                for (tname, tvalue) in t {
                    match (tname.as_str(), tvalue) {
                        ("feature", &toml::Value::String(ref s)) => {
                            feature = Some(s);
                        }
                        ("version", &toml::Value::String(ref s)) => {
                            version = Some(s);
                        }
                        ("name", &toml::Value::String(ref s)) => {
                            lib_name = Some(s);
                        }
                        ("feature-versions", &toml::Value::Table(ref feature_versions)) => {
                            for (k, v) in feature_versions {
                                match (k.as_str(), v) {
                                    (_, &toml::Value::String(ref feat_vers)) => {
                                        if has_feature(&k) {
                                            enabled_feature_versions.push(feat_vers);
                                        }
                                    }
                                    _ => bail!(
                                        "Unexpected feature-version key: {} type {}",
                                        k,
                                        v.type_str()
                                    ),
                                }
                            }
                        }
                        _ => bail!(
                            "Unexpected key {}.{}.{} type {}",
                            key,
                            name,
                            tname,
                            tvalue.type_str()
                        ),
                    }
                }
                if let Some(feature) = feature {
                    if !has_feature(feature) {
                        continue;
                    }
                }

                let version = {
                    // Pick the highest feature enabled version
                    if !enabled_feature_versions.is_empty() {
                        enabled_feature_versions.sort_by(|a, b| {
                            VersionCompare::compare(b, a)
                                .expect("failed to compare versions")
                                .ord()
                                .expect("invalid version")
                        });
                        Some(enabled_feature_versions[0])
                    } else {
                        version
                    }
                };

                (
                    lib_name.unwrap_or(name),
                    version.ok_or(format!("No version in {}.{}", key, name))?,
                )
            }
            _ => bail!("{}.{} not a string or table", key, name),
        };
        let library = Config::new().atleast_version(&version).probe(lib_name)?;
        libraries.insert(name.clone(), library);
    }
    Ok(libraries)
}
