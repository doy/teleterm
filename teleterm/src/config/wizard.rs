use crate::prelude::*;
use std::io::Write as _;

pub fn run() -> Result<Option<config::Config>> {
    println!("No configuration file found.");
    let run_wizard = prompt(
        "Would you like me to ask you some questions to generate one?",
    )?;
    if !run_wizard {
        let shouldnt_touch = prompt(
            "Would you like me to ask this question again in the future?",
        )?;
        if !shouldnt_touch {
            touch_config_file()?;
        }
        return Ok(None);
    }

    let connect_address =
        prompt_addr("Which server would you like to connect to?")?;
    let tls = prompt("Does this server require a TLS connection?")?;
    let auth_type = prompt_auth_type(
        "How would you like to authenticate to this server?",
    )?;

    write_config_file(&connect_address, tls, &auth_type).and_then(
        |config_filename| {
            Some(super::config_from_filename(&config_filename)).transpose()
        },
    )
}

fn touch_config_file() -> Result<()> {
    let config_filename = crate::dirs::Dirs::new()
        .config_file(super::CONFIG_FILENAME, false)
        .unwrap();
    std::fs::File::create(config_filename.clone()).context(
        crate::error::CreateFileSync {
            filename: config_filename.to_string_lossy(),
        },
    )?;
    Ok(())
}

fn write_config_file(
    connect_address: &str,
    tls: bool,
    auth_type: &str,
) -> Result<std::path::PathBuf> {
    let contents = format!(
        r#"[client]
connect_address = "{}"
tls = {}
auth = "{}"
"#,
        connect_address, tls, auth_type
    );
    let config_filename = crate::dirs::Dirs::new()
        .config_file(super::CONFIG_FILENAME, false)
        .unwrap();
    let mut file = std::fs::File::create(config_filename.clone()).context(
        crate::error::CreateFileSync {
            filename: config_filename.to_string_lossy(),
        },
    )?;
    file.write_all(contents.as_bytes())
        .context(crate::error::WriteFileSync)?;
    Ok(config_filename)
}

fn prompt(msg: &str) -> Result<bool> {
    print!("{} [y/n]: ", msg);
    std::io::stdout()
        .flush()
        .context(crate::error::FlushTerminal)?;
    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .context(crate::error::ReadTerminal)?;

    loop {
        match response.trim() {
            "y" | "yes" => {
                return Ok(true);
            }
            "n" | "no" => {
                return Ok(false);
            }
            _ => {
                print!("Please answer [y]es or [n]o: ");
                std::io::stdout()
                    .flush()
                    .context(crate::error::FlushTerminal)?;
                std::io::stdin()
                    .read_line(&mut response)
                    .context(crate::error::ReadTerminal)?;
            }
        }
    }
}

fn prompt_addr(msg: &str) -> Result<String> {
    loop {
        print!("{} [addr:port]: ", msg);
        std::io::stdout()
            .flush()
            .context(crate::error::FlushTerminal)?;
        let mut response = String::new();
        std::io::stdin()
            .read_line(&mut response)
            .context(crate::error::ReadTerminal)?;

        match response.trim() {
            addr if addr.contains(':') => {
                match super::to_connect_address(addr) {
                    Ok(..) => return Ok(addr.to_string()),
                    _ => {
                        println!("Couldn't parse '{}'.", addr);
                    }
                };
            }
            _ => {
                println!("Please include a port number.");
            }
        }
    }
}

fn prompt_auth_type(msg: &str) -> Result<String> {
    let auth_type_names: Vec<_> = crate::protocol::AuthType::iter()
        .map(crate::protocol::AuthType::name)
        .collect();

    loop {
        println!("{}", msg);
        println!("Options are:");
        for (i, name) in auth_type_names.iter().enumerate() {
            println!("{}: {}", i + 1, name);
        }
        print!("Choose [1-{}]: ", auth_type_names.len());
        std::io::stdout()
            .flush()
            .context(crate::error::FlushTerminal)?;
        let mut response = String::new();
        std::io::stdin()
            .read_line(&mut response)
            .context(crate::error::ReadTerminal)?;

        let num: Option<usize> = response.trim().parse().ok();
        if let Some(num) = num {
            if num > 0 && num <= auth_type_names.len() {
                let name = auth_type_names[num - 1];
                return Ok(name.to_string());
            }
        }

        println!("Invalid response '{}'", response.trim());
    }
}
