use error::{JudgeErrorKind, JudgeResult};
use testcase::{Case, Cases, TestCaseFilePath};
use util::{self, Foreach, UnwrapAsRefMut};

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use term::{Attr, color};


/// Tests for given test case file path, executaion command, and compilation command.
///
/// # Errors
///
/// Returns `Err` if compilation or execution command fails, or any test fails.
pub fn judge(
    testcase_path: TestCaseFilePath,
    run_command: CommandParameters,
    compilation_command: Option<CommandParameters>,
) -> JudgeResult<()> {
    fn judge_one(case: Case, run_command: CommandParameters) -> JudgeResult<JudgeOutput> {
        fn run_program(case: Case, run_command: CommandParameters) -> io::Result<JudgeOutput> {
            let (input, expected, timelimit) = case.into();
            let mut child = run_command.spawn_piped()?;
            let start = Instant::now();
            child.stdin.unwrap_as_ref_mut().write_all(input.as_bytes())?;

            let status = child.wait()?;
            let t = start.elapsed();
            let t = (1000000000 * t.as_secs() + t.subsec_nanos() as u64 + 999999) / 1000000;
            let stdout = util::string_from_read(child.stdout.unwrap())?;
            let stderr = util::string_from_read(child.stderr.unwrap())?;

            if timelimit.is_some() && t > timelimit.unwrap() {
                Ok(JudgeOutput::Tle(timelimit.unwrap(), input, expected))
            } else if status.success() && (expected.is_empty() || expected == stdout) {
                Ok(JudgeOutput::Ac(t, input, stdout, stderr))
            } else if status.success() {
                Ok(JudgeOutput::Wa(t, input, expected, stdout, stderr))
            } else {
                Ok(JudgeOutput::Re(t, input, expected, stdout, stderr, status))
            }
        }

        let (tx, rx) = mpsc::channel();
        let case_cloned = case.clone();
        thread::spawn(move || {
            let _ = tx.send(run_program(case_cloned, run_command));
        });
        Ok(if let (input, expected, Some(timelimit)) = case.into() {
            rx.recv_timeout(Duration::from_millis(timelimit + 50))
                .unwrap_or_else(|_| Ok(JudgeOutput::Tle(timelimit, input, expected)))?
        } else {
            rx.recv()??
        })
    }

    if let Some(compilation_command) = compilation_command {
        compilation_command.execute_as_compilation_command()?;
        println!("");
    }

    let cases = Cases::load(&testcase_path)?;
    let num_cases = cases.num_cases();
    let suf = if num_cases > 1 { "s" } else { "" };

    print_decorated!(Attr::Bold, Some(color::CYAN), "Test file:         ");
    println!("{}", testcase_path.build().display());
    run_command.print_args_and_working_dir();
    println!("Running {} test{}...", num_cases, suf);

    let outputs = cases
        .into_iter()
        .enumerate()
        .map(|(i, case)| {
            let output = judge_one(case, run_command.clone())?;
            output.print_title(i, num_cases);
            Ok(output)
        })
        .collect::<JudgeResult<Vec<_>>>()?;

    let num_failures = outputs.iter().filter(|output| !output.success()).count();
    if num_failures == 0 {
        println!("All of the {} test{} passed.", num_cases, suf);
        Ok(())
    } else {
        outputs.into_iter().enumerate().foreach(|(i, output)| {
            eprintln!("");
            output.eprint_title(i, num_cases);
            output.eprint_details();
        });
        bail!(JudgeErrorKind::TestFailed(num_failures))
    }
}


#[derive(Clone)]
pub struct CommandParameters {
    arg0: String,
    rest_args: Vec<String>,
    working_dir: PathBuf,
    src_and_bin: Option<(PathBuf, PathBuf)>,
}

impl CommandParameters {
    /// Constructs a new `CommandParameters`.
    ///
    /// Wraps `command` in `sh` or `cmd` if necessary.
    pub fn new(
        command: String,
        working_dir: PathBuf,
        src_and_bin: Option<(PathBuf, PathBuf)>,
    ) -> Self {
        if command
            .find(|c| "@#$^&*;|?\\<>()[]{}'\"".contains(c))
            .is_some()
        {
            let (arg0, c) = if cfg!(target_os = "windows") {
                ("cmd".to_owned(), "/C".to_owned())
            } else {
                ("sh".to_owned(), "-c".to_owned())
            };
            Self {
                arg0: arg0,
                rest_args: vec![c, command],
                working_dir: working_dir,
                src_and_bin: src_and_bin,
            }
        } else if command.find(char::is_whitespace).is_some() {
            let mut it = command.split_whitespace();
            let arg0 = it.next().unwrap_or_default().to_owned();
            let rest_args = it.map(str::to_owned).collect();
            Self {
                arg0: arg0,
                rest_args: rest_args,
                working_dir: working_dir,
                src_and_bin: src_and_bin,
            }
        } else {
            Self {
                arg0: command,
                rest_args: vec![],
                working_dir: working_dir,
                src_and_bin: src_and_bin,
            }
        }
    }

    fn print_args_and_working_dir(&self) {
        print_decorated!(Attr::Bold, Some(color::CYAN), "Command:           ");
        println!("{}", self.display_args());
        print_decorated!(Attr::Bold, Some(color::CYAN), "Working directory: ");
        println!("{}", self.working_dir.display());
    }

    fn execute_as_compilation_command(self) -> JudgeResult<()> {
        if let &Some((ref src, ref bin)) = &self.src_and_bin {
            if let (Ok(src_meta), Ok(bin_meta)) = (src.metadata(), bin.metadata()) {
                if let (Ok(t1), Ok(t2)) = (src_meta.modified(), bin_meta.modified()) {
                    if t1 < t2 {
                        println!("{} is up to date.", bin.display());
                        return Ok(());
                    }
                }
            } else if let Some(dir) = bin.parent() {
                if !dir.exists() {
                    fs::create_dir_all(dir)?;
                    println!("Created {}", dir.display());
                }
            }
        }
        self.print_args_and_working_dir();
        let status = Command::new(&self.arg0)
            .args(self.rest_args)
            .current_dir(self.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            bail!(JudgeErrorKind::CompilationFailure(status));
        }
    }

    fn spawn_piped(self) -> io::Result<Child> {
        Command::new(&self.arg0)
            .args(self.rest_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(self.working_dir)
            .spawn()
    }

    fn display_args(&self) -> String {
        let mut s: String = format!("{:?}", self.arg0);
        self.rest_args.iter().foreach(
            |arg| s += &format!(" {:?}", arg),
        );
        s
    }
}


enum JudgeOutput {
    // Each string may be empty.
    // (<elapsed>, <input>, <expected = stdout>, <stderr>)
    Ac(u64, String, String, String),
    // (<timelimit>, <input>, <expected>)
    Tle(u64, String, String),
    // (<elapsed>, <input>, <expected>, <stdout>, <stderr>)
    Wa(u64, String, String, String, String),
    // (<elapsed>, <input>, <expected>, <stdout>, <stderr>, <status>)
    Re(u64, String, String, String, String, ExitStatus),
}

impl fmt::Display for JudgeOutput {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            JudgeOutput::Ac(t, ..) => write!(f, "AC ({}ms)", t),
            JudgeOutput::Tle(t, ..) => write!(f, "TLE ({}ms)", t),
            JudgeOutput::Wa(t, ..) => write!(f, "WA ({}ms)", t),
            JudgeOutput::Re(t, .., status) => write!(f, "RE ({}, {}ms)", status, t),
        }
    }
}

impl JudgeOutput {
    fn success(&self) -> bool {
        match *self {
            JudgeOutput::Ac(..) => true,
            _ => false,
        }
    }

    fn print_title(&self, i: usize, n: usize) {
        (0..format!("{}", n).len() - format!("{}", i + 1).len()).foreach(|_| print!(" "));
        print_decorated!(Attr::Bold, None, "{}/{} ", i + 1, n);
        println_decorated!(Attr::Bold, Some(self.color()), "{}", self);
    }

    fn eprint_title(&self, i: usize, n: usize) {
        (0..format!("{}", n).len() - format!("{}", i + 1).len()).foreach(|_| eprint!(" "));
        eprint_decorated!(Attr::Bold, None, "{}/{} ", i + 1, n);
        eprintln_decorated!(Attr::Bold, Some(self.color()), "{}", self);
    }

    fn eprint_details(&self) {
        fn eprint_section(head: &'static str, s: &str) {
            eprintln_decorated!(Attr::Bold, Some(color::MAGENTA), "{}:", head);
            if s.is_empty() {
                eprintln_decorated!(Attr::Bold, Some(color::YELLOW), "EMPTY");
            } else {
                util::eprintln_trimming_last_newline(s);
            }
        }

        fn eprint_section_unless_empty(head: &'static str, s: &str) {
            if !s.is_empty() {
                eprintln_decorated!(Attr::Bold, Some(color::MAGENTA), "{}:", head);
                util::eprintln_trimming_last_newline(s);
            }
        }

        match *self {
            JudgeOutput::Ac(_, ref input, ref stdout, ref stderr) => {
                eprint_section("input", input);
                eprint_section("stdout", stdout);
                eprint_section_unless_empty("stderr", stderr);
            }
            JudgeOutput::Tle(_, ref input, ref expected) => {
                eprint_section("input", input);
                eprint_section_unless_empty("expected", expected);
            }
            JudgeOutput::Wa(_, ref input, ref expected, ref stdout, ref stderr) => {
                eprint_section("input", input);
                eprint_section("expected", expected);
                eprint_section("stdout", stdout);
                eprint_section_unless_empty("stderr", stderr);
            }
            JudgeOutput::Re(_, ref input, ref expected, ref stdout, ref stderr, _) => {
                eprint_section("input", input);
                eprint_section_unless_empty("expected", expected);
                eprint_section_unless_empty("stdout", stdout);
                eprint_section("stderr", stderr);
            }
        }
    }

    fn color(&self) -> u16 {
        match *self {
            JudgeOutput::Ac(..) => color::GREEN,
            JudgeOutput::Tle(..) => color::RED,
            JudgeOutput::Wa(..) => color::YELLOW,
            JudgeOutput::Re(..) => color::YELLOW,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::CommandParameters;

    use std::path::PathBuf;


    #[test]
    fn assert_commands_wrapped_in_sh_or_cmd() {
        fn make_command(command: &'static str) -> CommandParameters {
            CommandParameters::new(command.to_owned(), PathBuf::new(), None)
        }

        assert_eq!(
            "\"cargo\" \"build\" \"--release\"",
            make_command("cargo build --release").display_args()
        );

        let command = make_command("sleep 1 && cargo build --release").display_args();
        let display = if cfg!(target_os = "windows") {
            "\"cmd\" \"/C\" \"sleep 1 && cargo build --release\""
        } else {
            "\"sh\" \"-c\" \"sleep 1 && cargo build --release\""
        };
        assert_eq!(display, command);

        let command = make_command("echo 'Hello, World!'").display_args();
        let display = if cfg!(target_os = "windows") {
            "\"cmd\" \"/C\" \"echo \\'Hello, World!\\'\""
        } else {
            "\"sh\" \"-c\" \"echo \\'Hello, World!\\'\""
        };
        assert_eq!(display, command);
    }
}
