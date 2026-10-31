use clap::Parser;
use std::path::PathBuf;
use zn_assets::load_gltf;

#[derive(Parser, Debug)]
struct Cli {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    let mesh = match load_gltf(&cli.input) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("load error: {}", e);
            std::process::exit(1);
        }
    };
    // write bincode
    let data = bincode::serialize(&mesh).unwrap();
    std::fs::write(&cli.output, data).unwrap();
    println!("baked {} -> {}", cli.input.display(), cli.output.display());
}
