extern crate snowchains;

extern crate env_logger;
extern crate failure;
extern crate serde;
extern crate serde_yaml;
extern crate tempdir;

#[allow(dead_code)]
mod common;

use snowchains::app::App;
use snowchains::path::AbsPath;
use snowchains::ServiceName;

use std::io;

#[test]
#[ignore]
fn it_logins() {
    let _ = env_logger::try_init();
    let credentials = common::credentials_from_env_vars().unwrap();
    common::test_in_tempdir("it_logins", credentials, login);
}

#[test]
#[ignore]
#[should_panic(
    expected = "called `Result::unwrap()` on an `Err` value: Service(WrongCredentialsOnTest)"
)]
fn it_raises_an_error_when_login_fails() {
    let _ = env_logger::try_init();
    common::test_in_tempdir(
        "it_raises_an_error_if_the_credentials_is_wrong",
        common::dummy_credentials(),
        login,
    );
}

#[test]
#[ignore]
fn it_downloads_testcases() {
    let _ = env_logger::try_init();
    let credentials = common::credentials_from_env_vars().unwrap();
    common::test_in_tempdir(
        "it_downloads_test_cases_from_master",
        credentials,
        |app| -> Result<(), failure::Error> {
            static CONTEST: &str = "no";
            let wd = app.working_dir.clone();
            download(app, CONTEST, &["1", "2", "3"])?;
            confirm_num_cases(&wd, CONTEST, &[("1", 3), ("2", 4), ("3", 3)]);
            Ok(())
        },
    );
}

fn login(app: App<io::Empty, io::Sink, io::Sink>) -> snowchains::Result<()> {
    common::login(app, ServiceName::Yukicoder)
}

fn download(
    app: App<io::Empty, io::Sink, io::Sink>,
    contest: &str,
    problems: &[&str],
) -> snowchains::Result<()> {
    common::download(app, ServiceName::Yukicoder, contest, problems)
}

fn confirm_num_cases(wd: AbsPath, contest: &str, pairs: &[(&str, usize)]) {
    common::confirm_num_cases(wd, ServiceName::Yukicoder, contest, pairs)
}