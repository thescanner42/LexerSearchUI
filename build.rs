use std::{fs::OpenOptions, io::Write, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Config {
    build: Build,
}

#[derive(Debug, Deserialize)]
struct Build {
    public_url: String,
}

fn main() -> Result<(), String> {
    let toml = include_bytes!("Trunk.toml");
    let toml = str::from_utf8(toml).map_err(|e| e.to_string())?;
    let toml: Config = toml::from_str(toml).map_err(|e| e.to_string())?;

    let mut path = PathBuf::new();
    path.push("target");
    path.push("lexer-search-ui-public-url");

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| e.to_string())?;

    let write_bytes = toml
        .build
        .public_url
        .strip_prefix('/')
        .unwrap_or(&toml.build.public_url)
        .as_bytes();

    file.write(write_bytes).map_err(|e| e.to_string())?;
    file.write(b"#/").map_err(|e| e.to_string())?;

    println!("cargo:rerun-if-changed=Trunk.toml");
    Ok(())
}
