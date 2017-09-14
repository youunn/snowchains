pub mod atcoder;
pub mod atcoder_beta;
mod scraping_session;

use error::{ServiceErrorKind, ServiceResult};

use regex::Regex;
use rpassword;
use rprompt;
use std::io;
use std::io::BufRead;
use term::{Attr, color};


/// Reads username and password from stdin, showing the prompts on stderr.
///
/// The password is not hidden if `rpassword::prompt_password_stderr` fails.
fn read_username_and_password(username_prompt: &'static str) -> ServiceResult<(String, String)> {
    let username = rprompt::prompt_reply_stderr(&format!("{}: ", username_prompt))?;
    let password = rpassword::prompt_password_stderr("Password: ").or_else(
        |_| {
            eprintln_decorated!(Attr::Bold, Some(color::BRIGHT_MAGENTA), "FALLBACK");
            rprompt::prompt_reply_stderr("Password (not hidden): ")
        },
    )?;
    Ok((username, password))
}


/// Gets the value `x` if `Some(x) = o` and `!f(x)`.
///
/// # Errors
///
/// Returns `Err` if the above condition is not satisfied.
fn quit_on_failure<T>(o: Option<T>, f: for<'a> fn(&'a T) -> bool) -> ServiceResult<T> {
    if let Some(x) = o {
        if !f(&x) {
            return Ok(x);
        }
    }
    bail!(ServiceErrorKind::ScrapingFailed);
}
