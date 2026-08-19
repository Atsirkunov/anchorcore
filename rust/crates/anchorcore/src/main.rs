//! AnchorCore Rust binary — stub. See `rust/BACKLOG.md:1` and `docs/rust-port.md:1`.
//! The Python backend remains the shipped artifact; this crate will become
//! the drop-in replacement (same routes, same SQLite file).

use clap::Parser;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "8000")]
    port: u16,
    #[arg(long, default_value = "")]
    data_dir: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    println!("anchorcore (rust) stub — port {} data_dir {:?}", args.port, args.data_dir);
    println!("See rust/BACKLOG.md for the incremental plan. Python backend still ships.");
}
