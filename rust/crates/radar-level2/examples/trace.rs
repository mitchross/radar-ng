use clap::Parser;
#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    args: radar_level2::tracer::TraceArgs,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    radar_level2::tracer::run(Cli::parse().args).await
}
