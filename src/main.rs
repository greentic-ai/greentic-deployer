use clap::Parser;

use greentic_deployer::{
    apply,
    config::{CliArgs, DeployerConfig},
};

#[tokio::main]
async fn main() {
    let cli = CliArgs::parse();
    match DeployerConfig::from_env_and_args(cli) {
        Ok(config) => {
            if !config.config_warnings.is_empty() {
                eprintln!("configuration warnings:");
                for warning in &config.config_warnings {
                    eprintln!("- {warning}");
                }
            }
            if config.explain_config || config.explain_config_json {
                if config.explain_config_json {
                    let payload = serde_json::json!({
                        "config": &config.greentic,
                        "provenance": &config.provenance,
                        "warnings": &config.config_warnings,
                    });
                    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
                } else {
                    println!("greentic config (resolved):");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&config.greentic).unwrap()
                    );
                    println!("provenance: {:#?}", config.provenance);
                    if !config.config_warnings.is_empty() {
                        println!("warnings:");
                        for w in &config.config_warnings {
                            println!("- {w}");
                        }
                    }
                }
                return;
            }
            if let Err(err) = apply::run(config).await {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("configuration error: {err}");
            std::process::exit(1);
        }
    }
}
