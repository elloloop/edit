use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = edit_agent::parse_args(&args)?;
    match command {
        edit_agent::Command::Run(args) => edit_agent::run(args),
    }
}
