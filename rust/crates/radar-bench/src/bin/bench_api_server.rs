//! Runs `radar_api::serve` with configuration from the environment
//! (`LISTEN_ADDR`, `TILE_DIR`, `STATE_DIR`, `GRID_DIR`, `API_RATE_LIMIT_RPS`, ...).
//! Used only by bench/load/run_load.sh. Not part of the service.

use anyhow::Result;
use radar_api::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    eprintln!("bench_api_server listening on {}", config.listen_addr);
    radar_api::serve(config).await
}
