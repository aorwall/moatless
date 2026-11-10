//! Keep the `claude` argv within the kernel's and tmux's size limits: move an
//! oversized system prompt into a file, and name the largest argument when a
//! spawn still fails with `E2BIG`.

use std::io::Write;

use tempfile::NamedTempFile;

use crate::error::SdkError;

/// Linux's `MAX_ARG_STRLEN`: the largest single argv or env string `execve` accepts.
pub(crate) const MAX_ARG_STRLEN: usize = 32 * 4096;

/// Flags whose value moves to their `-file` twin when it exceeds the inline limit.
const SPILLABLE_FLAGS: [(&str, &str); 2] = [
    ("--append-system-prompt", "--append-system-prompt-file"),
    ("--system-prompt", "--system-prompt-file"),
];

/// Rewrite each system-prompt flag whose value is over `inline_limit` bytes to
/// its `-file` flag, pointing at a temp file holding the value.
///
/// The CLI reads the file at startup, so the caller keeps the returned handles
/// alive for the life of the process; dropping one deletes its file.
pub(crate) fn spill_oversized_prompts(
    args: &mut [String],
    inline_limit: usize,
) -> std::io::Result<Vec<NamedTempFile>> {
    let mut files = Vec::new();
    for i in 0..args.len().saturating_sub(1) {
        let Some((_, file_flag)) = SPILLABLE_FLAGS.iter().find(|(flag, _)| *flag == args[i]) else {
            continue;
        };
        if args[i + 1].len() <= inline_limit {
            continue;
        }
        let mut file = tempfile::Builder::new()
            .prefix("claude-system-prompt-")
            .suffix(".md")
            .tempfile()?;
        file.write_all(args[i + 1].as_bytes())?;
        file.flush()?;
        args[i] = (*file_flag).to_string();
        args[i + 1] = file.path().display().to_string();
        files.push(file);
    }
    Ok(files)
}

/// Map a spawn error to [`SdkError::ArgumentTooLong`] when it is `E2BIG`,
/// naming the largest argv value (by its flag) or env variable.
pub(crate) fn spawn_error<'a>(
    error: std::io::Error,
    args: &[String],
    env: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> SdkError {
    if error.kind() != std::io::ErrorKind::ArgumentListTooLong {
        return SdkError::SpawnError(error);
    }
    let largest_arg = args
        .iter()
        .enumerate()
        .max_by_key(|(_, a)| a.len())
        .map(|(i, value)| {
            let name = match i.checked_sub(1).map(|p| &args[p]) {
                Some(flag) if flag.starts_with("--") => format!("value of `{flag}`"),
                _ => format!("argument {i}"),
            };
            (name, value.len())
        });
    let largest_env = env
        .into_iter()
        .map(|(k, v)| (format!("env `{k}`"), k.len() + v.len() + 2))
        .max_by_key(|(_, len)| *len);
    let (arg, bytes) = largest_arg
        .into_iter()
        .chain(largest_env)
        .max_by_key(|(_, len)| *len)
        .unwrap_or_else(|| ("argument list".to_string(), 0));
    SdkError::ArgumentTooLong {
        arg,
        bytes,
        total_bytes: args.iter().map(|a| a.len() + 1).sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt_args(prompt: String) -> Vec<String> {
        vec![
            "--verbose".to_string(),
            "--append-system-prompt".to_string(),
            prompt,
            "--plugin-dir".to_string(),
            "/p".to_string(),
        ]
    }

    #[test]
    fn a_prompt_over_the_limit_moves_to_a_file() {
        let prompt = "x".repeat(MAX_ARG_STRLEN + 1);
        let mut args = prompt_args(prompt.clone());

        let files = spill_oversized_prompts(&mut args, 1024).unwrap();

        assert_eq!(args[1], "--append-system-prompt-file");
        assert_eq!(std::fs::read_to_string(&args[2]).unwrap(), prompt);
        assert_eq!(files.len(), 1);
        assert_eq!(&args[3..], ["--plugin-dir", "/p"]);
        assert!(args.iter().all(|a| a.len() < 1024));
    }

    #[test]
    fn a_prompt_within_the_limit_stays_inline() {
        let mut args = prompt_args("Be brief.".to_string());

        let files = spill_oversized_prompts(&mut args, 1024).unwrap();

        assert!(files.is_empty());
        assert_eq!(args, prompt_args("Be brief.".to_string()));
    }

    #[test]
    fn a_replacement_prompt_moves_to_its_own_file_flag() {
        let mut args = vec!["--system-prompt".to_string(), "y".repeat(2048)];

        let _files = spill_oversized_prompts(&mut args, 1024).unwrap();

        assert_eq!(args[0], "--system-prompt-file");
    }

    #[test]
    fn e2big_names_the_flag_whose_value_is_largest() {
        let args = prompt_args("x".repeat(132_262));
        let error = std::io::Error::from(std::io::ErrorKind::ArgumentListTooLong);

        let message = spawn_error(error, &args, [("HOME", "/root")]).to_string();

        assert!(
            message.contains("value of `--append-system-prompt` is 132262 bytes"),
            "{message}"
        );
    }

    #[test]
    fn e2big_names_an_env_variable_larger_than_every_argument() {
        let big = "v".repeat(200_000);
        let error = std::io::Error::from(std::io::ErrorKind::ArgumentListTooLong);

        let message =
            spawn_error(error, &prompt_args("p".into()), [("TOKENS", big.as_str())]).to_string();

        assert!(message.contains("env `TOKENS`"), "{message}");
    }

    #[test]
    fn other_spawn_errors_pass_through() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);

        let mapped = spawn_error(error, &[], []);

        assert!(matches!(mapped, SdkError::SpawnError(_)));
    }
}
