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
