use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    checkghossversion().map_err(|e| {
        eprintln!("failed to execute the checkghossversion: {e:?}");
        e
    })?;
    pwnedpassword().map_err(|e| {
        eprintln!("failed to execute the pwnedpassword: {e:?}");
        e
    })?;

    checksqlite()?;

    Ok(())
}

fn checkghossversion() -> anyhow::Result<()> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let asset_path = Path::new(&manifest_dir).join("asset");

    let mut methods = Vec::new();
    let mut file_string = String::new();
    for file_name in ["fragment_release", "fragment_tag"].iter() {
        let file_path = asset_path.join(format!("{file_name}.graphql"));
        let mut file = match File::open(&file_path) {
            Ok(ok) => ok,
            Err(e) => anyhow::bail!("Failed to open file: {:?}, {:?}", file_path, e),
        };
        file_string.clear();
        file.read_to_string(&mut file_string)?;
        methods.push(
            format!(
                r##"pub fn get_{}() -> &'static str {{
    r#"{}"#
}}
"##,
                file_name, &file_string
            )
            .into_bytes(),
        );
    }

    let out_dir = env::var("OUT_DIR")?;
    let mut checkghossversion_file =
        File::create(Path::new(&out_dir).join("checkghossversion_token.rs"))?;

    for method_str in &methods {
        checkghossversion_file.write_all(method_str)?;
    }
    Ok(())
}

fn pwnedpassword() -> anyhow::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if !std::process::Command::new("which")
            .args(["pkg-config"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()?
            .status
            .success()
        {
            anyhow::bail!("Need to install the pkg-config")
        }

        let status = std::process::Command::new("pkg-config")
            .args(["--exists", "sqlite3"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()?
            .status;

        if !status.success() {
            let command_help = if cfg!(target_os = "linux") {
                "sudo apt install libsqlite3-dev"
            } else {
                "sudo port install sqlite3"
            };

            anyhow::bail!("Need to install the sqlite3 via \"{}\"", command_help)
        }
    }
    Ok(())
}

fn checksqlite() -> anyhow::Result<()> {
    if env::var("CARGO_CFG_WINDOWS").is_ok() {
        // bundled-windows.
        return Ok(());
    }

    pkg_config::Config::new()
        // for upsert.
        .atleast_version("3.22.0")
        .probe("sqlite3")?;
    Ok(())
}
