use std::path::PathBuf;

use product_runtime::{start_product_runtime, ProductRuntimeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if product_runtime::run_kernel_worker_from_args_if_requested() {
        return Ok(());
    }
    let mut args = std::env::args().skip(1);
    let mut workspace = std::env::current_dir()?;
    let mut host = "127.0.0.1".to_string();
    let mut port = 8765_u16;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workspace" => {
                if let Some(value) = args.next() {
                    workspace = PathBuf::from(value);
                }
            }
            "--host" => {
                if let Some(value) = args.next() {
                    host = value;
                }
            }
            "--port" => {
                if let Some(value) = args.next() {
                    port = value.parse()?;
                }
            }
            _ => {}
        }
    }
    let handle = start_product_runtime(ProductRuntimeConfig {
        bind_host: host,
        preferred_port: port,
        app_config_root: None,
        app_state_root: None,
        initial_workspace: workspace,
        register_initial_workspace: true,
        runtime_token: None,
        allowed_origins: Vec::new(),
    })
    .await?;
    println!(
        "{}",
        serde_json::to_string(&handle.diagnostics_snapshot()).unwrap_or_default()
    );
    tokio::signal::ctrl_c().await?;
    handle.shutdown().await;
    Ok(())
}
