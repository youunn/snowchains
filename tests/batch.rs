use snowchains::app::{App, Modify, Opt};
use snowchains::errors::{JudgeError, JudgeErrorKind};
use snowchains::path::AbsPathBuf;
use snowchains::service::{Credentials, ServiceName};
use snowchains::terminal::{AnsiColorChoice, Term, TermImpl};
use snowchains::testsuite::SuiteFileExtension;

use failure::Fallible;
use if_chain::if_chain;
use tempdir::TempDir;

use std::path::Path;
use std::time::Duration;

#[test]
fn it_works_for_atcoder_practice_a() -> Fallible<()> {
    static SUITE: &str = r#"---
type: batch
match: exact
cases:
  - name: Sample 1
    in: |
      1
      2 3
      test
    out: |
      6 test
  - name: Sample 2
    in: |
      72
      128 256
      myonmyon
    out: |
      456 myonmyon
"#;
    static CODE: &str = r#"use std::io::{self, Read};

fn main() {
    let mut input = "".to_owned();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut input = input.split(char::is_whitespace);
    let a = input.next().unwrap().parse::<u32>().unwrap();
    let b = input.next().unwrap().parse::<u32>().unwrap();
    let c = input.next().unwrap().parse::<u32>().unwrap();
    let s = input.next().unwrap();
    println!("{} {}", a + b + c, s);
}
"#;
    static INVLID_CODE: &str = "print('Hello!')";
    static WRONG_CODE: &str = "fn main() {}";
    static FREEZING_CODE: &str = r#"use std::thread;
use std::time::Duration;

fn main() {
    thread::sleep(Duration::from_secs(10));
}
"#;

    let _ = env_logger::try_init();

    let tempdir = TempDir::new("batch_it_works")?;

    let dir = tempdir.path().join("atcoder").join("practice");
    let src_dir = dir.join("rs").join("src").join("bin");
    let src_path = src_dir.join("a.rs");
    let suite_dir = dir.join("tests");
    let suite_path = suite_dir.join("a.yaml");

    std::fs::write(
        tempdir.path().join("snowchains.yaml"),
        include_bytes!("./snowchains.yaml").as_ref(),
    )?;
    std::fs::create_dir_all(&src_dir)?;
    std::fs::create_dir_all(&suite_dir)?;
    std::fs::write(&suite_path, SUITE)?;

    let mut app = App {
        working_dir: AbsPathBuf::try_new(tempdir.path().to_owned()).unwrap(),
        credentials: Credentials::default(),
        term: TermImpl::null(),
    };

    app.test(&src_path, CODE)?;

    if_chain! {
        let err = app.test(&src_path, INVLID_CODE).unwrap_err();
        if let snowchains::Error::Judge(JudgeError::Context(ctx)) = &err;
        if let JudgeErrorKind::Build { .. } = ctx.get_context();
        then {} else { return Err(err.into()) }
    }

    if_chain! {
        let err = app.test(&src_path, WRONG_CODE).unwrap_err();
        if let snowchains::Error::Judge(JudgeError::Context(ctx)) = &err;
        if let JudgeErrorKind::TestFailed(2, 2) = ctx.get_context();
        then {} else { return Err(err.into()) }
    }

    app.modify_timelimit(Duration::from_millis(100))?;

    if_chain! {
        let err = app.test(&src_path, FREEZING_CODE).unwrap_err();
        if let snowchains::Error::Judge(JudgeError::Context(ctx)) = &err;
        if let JudgeErrorKind::TestFailed(2, 2) = ctx.get_context();
        then { Ok(()) } else { Err(err.into()) }
    }
}

trait AppExt {
    fn test(&mut self, src_path: &Path, code: &str) -> snowchains::Result<()>;
    fn modify_timelimit(&mut self, timelimit: Duration) -> snowchains::Result<()>;
}

impl<T: Term> AppExt for App<T> {
    fn test(&mut self, src_path: &Path, code: &str) -> snowchains::Result<()> {
        std::fs::write(src_path, code)?;
        self.run(Opt::Judge {
            force_compile: false,
            service: Some(ServiceName::Atcoder),
            contest: Some("practice".to_owned()),
            language: Some("rust".to_owned()),
            jobs: None,
            color_choice: AnsiColorChoice::Never,
            problem: "a".to_owned(),
        })
    }

    fn modify_timelimit(&mut self, timelimit: Duration) -> snowchains::Result<()> {
        self.run(Opt::Modify(Modify::Timelimit {
            service: Some(ServiceName::Atcoder),
            contest: Some("practice".to_owned()),
            color_choice: AnsiColorChoice::Never,
            problem: "a".to_owned(),
            extension: SuiteFileExtension::Yaml,
            timelimit: Some(timelimit),
        }))
    }
}
