//! Assembling the one command line the shell is asked to run.
//!
//! Separated from the adapter because this is where the adapter can be subtly wrong in a way no
//! integration test reliably catches: a case file is data, and an argument that happens to contain
//! a shell separator must stay one argument.

/// Wraps `argument` so the shell treats it as one literal word.
///
/// Single quotes make everything inert; the only character needing care is the single quote itself,
/// which closes the run, contributes an escaped quote, and reopens.
pub fn quote(argument: &str) -> String {
    format!("'{}'", argument.replace('\'', r"'\''"))
}

/// The line that sources each file in order, then calls the function.
///
/// Joined with `;` rather than `&&`: a source that fails leaves the function undefined, and the
/// resulting "command not found" on standard error with a non-zero exit says more to the reader
/// than a line that stops silently.
///
/// The function name is quoted like its arguments. Both shells strip the quotes before resolving
/// the command word, so a function still runs — and a name arriving from a case file cannot become
/// a substitution.
pub fn assemble(sources: &[String], call: &[String]) -> String {
    let mut parts: Vec<String> = sources
        .iter()
        .map(|file| format!("source {}", quote(file)))
        .collect();

    if !call.is_empty() {
        parts.push(
            call.iter()
                .map(|word| quote(word))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_argument_with_a_single_quote_survives_quoting() {
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn an_argument_that_would_run_a_second_command_is_inert() {
        let line = assemble(&[], &["echo".to_string(), "hi; rm -rf /".to_string()]);
        assert!(
            line.contains(r"'hi; rm -rf /'"),
            "a case is data, not a script. An argument containing a separator must reach the \
             function as one argument, or a case file becomes a way to run anything: {line}"
        );
    }

    #[test]
    fn the_sources_come_before_the_call_and_in_order() {
        let line = assemble(
            &[
                "functions/ui.zsh".to_string(),
                "functions/kube.zsh".to_string(),
            ],
            &["kube_switch".to_string(), "staging".to_string()],
        );
        assert_eq!(
            line,
            r"source 'functions/ui.zsh'; source 'functions/kube.zsh'; 'kube_switch' 'staging'"
        );
    }

    #[test]
    fn sourcing_with_nothing_to_call_is_a_line_that_only_loads() {
        assert_eq!(assemble(&["rc.zsh".to_string()], &[]), "source 'rc.zsh'");
    }
}
