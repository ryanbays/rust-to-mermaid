mod build_diagram;

use clap::Parser;
use std::path::PathBuf;

use crate::build_diagram::DiagramConfig;

/// CLI configuration for rust-to-mermaid.
#[derive(Debug, Parser)]
#[command(
    name = "rust-to-mermaid",
    about = "Generate Mermaid diagrams from Rust code"
)]
struct Cli {
    /// Source Rust file or directory
    #[arg(short, long, value_name = "SRC", default_value = "src")]
    src: PathBuf,

    /// Output directory for generated diagrams
    #[arg(short, long, value_name = "OUT", default_value = "diagrams")]
    out: PathBuf,

    /// Main diagram title
    #[arg(long, default_value = "Project")]
    main_title: String,

    /// Tests diagram title
    #[arg(long, default_value = "Project Tests")]
    tests_title: String,

    /// Layout engine (e.g. elk, dagre)
    #[arg(long, default_value = "elk")]
    layout: String,

    /// Mermaid theme (e.g. default, dark, forest)
    #[arg(long, default_value = "dark")]
    theme: String,

    /// ELK node placement strategy
    #[arg(long, default_value = "BRANDES_KOEPF")]
    elk_node_placement: String,
}

fn main() {
    let cli = Cli::parse();

    let src = cli.src;
    let out = cli.out;

    let config = DiagramConfig {
        main_title: &cli.main_title,
        tests_title: &cli.tests_title,
        layout: &cli.layout,
        theme: &cli.theme,
        elk_node_placement: &cli.elk_node_placement,
        src_dir: &src,
        out_dir: &out,
    };

    if let Err(e) = build_diagram::generate_diagrams_with_config(&config) {
        eprintln!("Error generating diagrams: {e}");
        std::process::exit(1);
    }
}

