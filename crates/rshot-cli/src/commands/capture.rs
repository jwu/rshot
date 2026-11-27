use crate::{error::CliError, output::CommandOutput};

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_: Args) -> Result<CommandOutput, CliError> {
    Err(CliError::not_implemented("capture"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_not_implemented() {
        let error = run(Args {}).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert_eq!(error.kind, "not_implemented");
        assert!(error.hint.is_none());
    }
}
