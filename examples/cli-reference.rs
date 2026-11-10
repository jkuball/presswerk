//! Print the CLI reference as JSON for the documentation site.

fn main() -> serde_json::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&presswerk::cli::reference())?
    );
    Ok(())
}
