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
            if config.explain_config {
                if let Ok(payload) = serde_json::to_string_pretty(&config.greentic) {
                    println!("{payload}");
                } else {
                    println!("{:#?}", config.greentic);
                }
                println!("provenance: {:#?}", config.provenance);
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
