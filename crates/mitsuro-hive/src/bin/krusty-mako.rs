//! Deprecated daemon-name bridge for mixed Mitsuro installations.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    eprintln!("`krusty-mako` is deprecated; use `mitsuro-hive`");
    let current = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("failed to locate the Mitsuro Hive executable: {error}");
            return ExitCode::FAILURE;
        }
    };
    let executable_name = if cfg!(windows) {
        "mitsuro-hive.exe"
    } else {
        "mitsuro-hive"
    };
    let canonical = current.with_file_name(executable_name);
    let mut command = Command::new(&canonical);
    command.args(std::env::args_os().skip(1));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        eprintln!(
            "failed to replace compatibility process with {}: {error}",
            canonical.display()
        );
        ExitCode::FAILURE
    }

    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => {
            eprintln!(
                "failed to launch canonical Mitsuro Hive daemon at {}: {error}",
                canonical.display()
            );
            ExitCode::FAILURE
        }
    }
}
