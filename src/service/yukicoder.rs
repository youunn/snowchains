use console::Palette;
use errors::{ServiceError, ServiceResult, SessionResult, SubmitError};
use service::downloader::ZipDownloader;
use service::session::{HasSession, HttpSession};
use service::{
    Contest, DownloadProp, PrintTargets as _PrintTargets, RevelSession, SessionProp, SubmitProp,
    TryIntoDocument as _TryIntoDocument,
};
use testsuite::{SuiteFilePath, TestSuite};

use cookie::Cookie;
use regex::Regex;
use reqwest::header::Location;
use reqwest::{multipart, StatusCode};
use select::document::Document;
use select::predicate::{Attr, Class, Name, Predicate as _Predicate, Text};

use std::borrow::Cow;
use std::fmt;
use std::io::{BufRead, Write};
use std::time::Duration;

pub(crate) fn login(
    sess_prop: SessionProp<impl BufRead, impl Write, impl Write>,
) -> ServiceResult<()> {
    Yukicoder::new(sess_prop)?.login(true)
}

pub(crate) fn download(
    sess_prop: SessionProp<impl BufRead, impl Write, impl Write>,
    download_prop: DownloadProp<String>,
) -> ServiceResult<()> {
    let download_prop = download_prop.parse_contest();
    download_prop.write_targets(sess_prop.console.stdout())?;
    let timeout = sess_prop.timeout;
    Yukicoder::new(sess_prop)?.download(&download_prop, timeout)
}

pub(crate) fn submit(
    sess_prop: SessionProp<impl BufRead, impl Write, impl Write>,
    submit_prop: SubmitProp<String>,
) -> ServiceResult<()> {
    let submit_prop = submit_prop.parse_contest();
    Yukicoder::new(sess_prop)?.submit(&submit_prop)
}

struct Yukicoder<'a, I: BufRead + 'a, O: Write + 'a, E: Write + 'a> {
    session: HttpSession<'a, I, O, E>,
    username: Username,
    credential: RevelSession,
}

impl<'a, I: BufRead, O: Write, E: Write> HasSession<'a> for Yukicoder<'a, I, O, E> {
    type Stdin = I;
    type Stdout = O;
    type Stderr = E;

    fn session<'b>(&'b mut self) -> &'b mut HttpSession<'a, I, O, E>
    where
        'a: 'b,
    {
        &mut self.session
    }
}

impl<'a, I: BufRead, O: Write, E: Write> Yukicoder<'a, I, O, E> {
    fn new(sess_prop: SessionProp<'a, I, O, E>) -> SessionResult<Self> {
        let credential = sess_prop.credentials.yukicoder.clone();
        let session = sess_prop.start_session()?;
        Ok(Self {
            session,
            username: Username::None,
            credential,
        })
    }

    fn login(&mut self, assure: bool) -> ServiceResult<()> {
        if let RevelSession::Some(revel_session) = self.credential.clone() {
            if !self.confirm_revel_session(revel_session.as_ref().clone())? {
                return Err(ServiceError::WrongCredentialsOnTest);
            }
        }
        self.fetch_username()?;
        if self.username.name().is_none() {
            let mut first = true;
            loop {
                if first {
                    if !assure && !self.console().ask_yes_or_no("Login? ", true)? {
                        break;
                    }
                    writeln!(
                        self.stdout(),
                        "\nInput \"REVEL_SESSION\".\n\n\
                         Firefox: sqlite3 ~/path/to/cookies.sqlite 'SELECT value FROM moz_cookies \
                         WHERE baseDomain=\"yukicoder.me\" AND name=\"REVEL_SESSION\"'\n\
                         Chrome: chrome://settings/cookies/detail?site=yukicoder.me&search=cookie\n"
                    )?;
                    self.stdout().flush()?;
                    first = false;
                }
                let revel_session = self.console().prompt_password_stderr("REVEL_SESSION: ")?;
                if self.confirm_revel_session(revel_session)? {
                    break;
                } else {
                    writeln!(self.stderr(), "Wrong \"REVEL_SESSION\".")?;
                    self.stderr().flush()?;
                }
            }
        }
        let username = self.username.clone();
        writeln!(self.stdout(), "Username: {}", username)?;
        self.stdout().flush()?;
        Ok(())
    }

    fn confirm_revel_session(&mut self, revel_session: String) -> ServiceResult<bool> {
        self.session.clear_cookies()?;
        let cookie = Cookie::new("REVEL_SESSION", revel_session);
        self.session.insert_cookie(cookie)?;
        self.fetch_username()?;
        Ok(self.username.name().is_some())
    }

    fn fetch_username(&mut self) -> SessionResult<()> {
        self.username = self.get("/").recv_html()?.extract_username();
        Ok(())
    }

    fn download(
        &mut self,
        download_prop: &DownloadProp<YukicoderContest>,
        timeout: Option<Duration>,
    ) -> ServiceResult<()> {
        let DownloadProp {
            contest,
            problems,
            download_dir,
            extension,
            open_browser,
        } = download_prop;
        self.login(false)?;
        let scrape =
            |document: &Document, problem: &str| -> ServiceResult<(TestSuite, SuiteFilePath)> {
                let suite = document.extract_samples()?;
                let path = SuiteFilePath::new(download_dir, problem, *extension);
                Ok((suite, path))
            };
        let (mut outputs, mut nos) = (vec![], vec![]);
        match (contest, problems.as_ref()) {
            (YukicoderContest::No, None) => return Err(ServiceError::PleaseSpecifyProblems),
            (YukicoderContest::No, Some(problems)) => {
                let (mut not_found, mut not_public) = (vec![], vec![]);
                for problem in problems {
                    let url = format!("/problems/no/{}", problem);
                    let res = self.get(&url).acceptable(&[200, 404]).send()?;
                    let status = res.status();
                    let document = res.try_into_document()?;
                    let public = match document.find(Attr("id", "content").child(Text)).next() {
                        None => true,
                        Some(t) => !t.text().contains("非表示"),
                    };
                    if status == StatusCode::NotFound {
                        not_found.push(problem);
                    } else if !public {
                        not_public.push(problem);
                    } else {
                        outputs.push(scrape(&document, problem).map(|(s, p)| (url, s, p))?);
                        nos.push(Cow::from(problem.as_str()));
                    }
                }
                let mut stderr = self.stderr();
                if !not_found.is_empty() {
                    writeln!(stderr.plain(Palette::Warning), "Not found: {:?}", not_found)?;
                    stderr.flush()?;
                }
                if !not_public.is_empty() {
                    writeln!(
                        stderr.plain(Palette::Warning),
                        "Not public: {:?}",
                        not_found
                    )?;
                    stderr.flush()?;
                }
            }
            (YukicoderContest::Contest(contest), problems) => {
                let target_problems = self
                    .get(&format!("/contests/{}", contest))
                    .recv_html()?
                    .extract_problems()?;
                let mut outputs = vec![];
                for (name, href) in target_problems {
                    if problems.is_none() || problems.as_ref().unwrap().contains(&name) {
                        let name = name.to_lowercase();
                        let document = self.get(&href).recv_html()?;
                        outputs.push(scrape(&document, &name).map(|(s, p)| (href, s, p))?);
                        nos.push(Cow::from(name));
                    }
                }
            }
        }
        let nos = self.filter_solved(&nos)?;
        for (_, suite, path) in &outputs {
            suite.save(path, self.stdout())?;
        }
        self.stdout().flush()?;
        if !nos.is_empty() {
            static URL_PREF: &str = "https://yukicoder.me/problems/no/";
            static URL_SUF: &str = "/testcase.zip";
            let cookie = self.session.cookies_to_header();
            ZipDownloader {
                out: self.stdout().inner(),
                url_pref: URL_PREF,
                url_suf: URL_SUF,
                download_dir,
                names: &nos,
                timeout,
                cookie: cookie.as_ref(),
            }.download()?;
        }
        if *open_browser {
            for (url, _, _) in &outputs {
                self.session.open_in_browser(url)?;
            }
        }
        Ok(())
    }

    fn submit(&mut self, prop: &SubmitProp<YukicoderContest>) -> ServiceResult<()> {
        let SubmitProp {
            contest,
            problem,
            lang_id,
            src_path,
            replacer,
            open_browser,
            skip_checking_if_accepted,
        } = prop;
        self.login(true)?;
        let code = ::fs::read_to_string(src_path)?;
        let code = match replacer {
            Some(replacer) => replacer.replace_from_local_to_submission(&problem, &code)?,
            None => code,
        };
        let mut url = match contest {
            YukicoderContest::No => format!("/problems/no/{}", problem),
            YukicoderContest::Contest(contest) => self
                .get(&format!("/contests/{}", contest))
                .recv_html()?
                .extract_problems()?
                .into_iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case(problem))
                .map(|(_, href)| href)
                .next()
                .ok_or_else(|| SubmitError::NoSuchProblem(problem.clone()))?,
        };
        url += "/submit";
        let no = {
            lazy_static! {
                static ref NO: Regex =
                    Regex::new(r"\A(https://yukicoder\.me)?/problems/no/(\d+)/submit\z").unwrap();
            }
            NO.captures(&url).map(|caps| caps[2].to_owned())
        };
        if let Some(no) = no {
            if !(self.filter_solved(&[no])?.is_empty() || *skip_checking_if_accepted) {
                return Err(ServiceError::AlreadyAccepted);
            }
        }
        let document = self.get(&url).recv_html()?;
        let token = document.extract_csrf_token_from_submit_page()?;
        let form = multipart::Form::new()
            .text("csrf_token", token)
            .text("lang", lang_id.clone())
            .text("source", code.clone())
            .text("submit", "提出する");
        let url = document.extract_url_from_submit_page()?;
        let res = self.post(&url).send_multipart(form)?;
        let location = match res.headers().get::<Location>() {
            None => None,
            Some(location) => Some(self.session.resolve_url(&location)?),
        };
        if let Some(location) = location.as_ref() {
            if location
                .as_str()
                .starts_with("https://yukicoder.me/submissions/")
            {
                writeln!(self.stdout(), "Success: {}", location)?;
                self.stdout().flush()?;
                if *open_browser {
                    self.session.open_in_browser(location.as_str())?;
                }
                return Ok(());
            }
        }
        Err(SubmitError::Rejected(lang_id.clone(), code.len(), location).into())
    }

    fn filter_solved<'b>(
        &mut self,
        nos: &'b [impl 'b + AsRef<str>],
    ) -> ServiceResult<Vec<&'b str>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Problem {
            no: u64,
        }

        let (session, username) = (&mut self.session, &self.username);
        if let Some(username) = username.name() {
            let url = format!("/api/v1/solved/name/{}", username);
            let solved_nos = session
                .get(&url)
                .send()?
                .json::<Vec<Problem>>()?
                .into_iter()
                .map(|problem| problem.no.to_string())
                .collect::<Vec<_>>();
            Ok(nos
                .iter()
                .map(AsRef::as_ref)
                .filter(|no1| solved_nos.iter().any(|no2| no1 == no2))
                .collect())
        } else {
            Ok(vec![])
        }
    }
}

enum YukicoderContest {
    No,
    Contest(String),
}

impl fmt::Display for YukicoderContest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            YukicoderContest::No => write!(f, "no"),
            YukicoderContest::Contest(contest) => write!(f, "{}", contest),
        }
    }
}

impl Contest for YukicoderContest {
    fn from_string(s: String) -> Self {
        if s.eq_ignore_ascii_case("no") {
            YukicoderContest::No
        } else {
            YukicoderContest::Contest(s)
        }
    }
}

#[derive(Clone, Debug)]
enum Username {
    None,
    // /public/img/anony.png
    Anonymous,
    // https://avatars2.githubusercontent.com/...
    Github(String),
    // ?
    ProbablyTwitter(String),
}

impl Username {
    fn name(&self) -> Option<&str> {
        match self {
            Username::None | Username::Anonymous => None,
            Username::Github(s) | Username::ProbablyTwitter(s) => Some(&s),
        }
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Username::None => write!(f, "<not logged in>"),
            Username::Anonymous => write!(f, "<anonymous>"),
            Username::Github(s) => write!(f, "{} (GitHub)", s),
            Username::ProbablyTwitter(s) => write!(f, "{} (probably Twitter)", s),
        }
    }
}

trait Extract {
    fn extract_username(&self) -> Username;
    fn extract_samples(&self) -> ServiceResult<TestSuite>;
    fn extract_problems(&self) -> ServiceResult<Vec<(String, String)>>;
    fn extract_csrf_token_from_submit_page(&self) -> ServiceResult<String>;
    fn extract_url_from_submit_page(&self) -> ServiceResult<String>;
}

impl Extract for Document {
    fn extract_username(&self) -> Username {
        let extract = || {
            let a = self.find(Attr("id", "usermenu").child(Name("a"))).next()?;
            let name = a.find(Text).next()?.text();
            let img = a.find(Name("img")).next()?;
            let src = img.attr("src")?;
            Some(if src == "/public/img/anony.png" {
                Username::Anonymous
            } else if src.starts_with("https://avatars2.githubusercontent.com") {
                Username::Github(name)
            } else {
                Username::ProbablyTwitter(name)
            })
        };
        extract().unwrap_or(Username::None)
    }

    fn extract_samples(&self) -> ServiceResult<TestSuite> {
        // TODO:
        // - https://yukicoder.me/problems/no/188
        // - https://yukicoder.me/problems/no/192
        let extract = || {
            lazy_static! {
                static ref TIMELIMIT: Regex = Regex::new(
                    "\\A / 実行時間制限 : 1ケース (\\d)\\.(\\d{3})秒 / メモリ制限 : \\d+ MB / \
                     通常問題\n\t*\\z"
                ).unwrap();
            }
            let timelimit = self
                .find(Attr("id", "content").child(Name("p")).child(Text))
                .filter_map(|t| {
                    TIMELIMIT.captures(&t.text()).map(|cs| {
                        let (s, m) = (cs[1].parse::<u64>().unwrap(), cs[2].parse::<u64>().unwrap());
                        Duration::from_millis(1000 * s + m)
                    })
                }).next()?;
            let mut samples = vec![];
            let predicate = Attr("id", "content")
                .child(Name("div").and(Class("block")))
                .child(Name("div").and(Class("sample")))
                .child(Name("div").and(Class("paragraph")));
            for paragraph in self.find(predicate) {
                let pres = paragraph.find(Name("pre").child(Text)).collect::<Vec<_>>();
                ensure_opt!(pres.len() == 2);
                samples.push((pres[0].text(), pres[1].text()));
            }
            ensure_opt!(!samples.is_empty());
            Some(TestSuite::simple(timelimit, None, None, samples))
        };
        extract().ok_or(ServiceError::Scrape)
    }

    fn extract_problems(&self) -> ServiceResult<Vec<(String, String)>> {
        let extract = || {
            let mut problems = vec![];
            let predicate = Attr("id", "content")
                .child(Name("div").and(Class("left")))
                .child(Name("table").and(Class("table")))
                .child(Name("tbody"))
                .child(Name("tr"));
            for tr in self.find(predicate) {
                let name = tr.find(Name("td")).nth(0)?.text();
                let href = tr
                    .find(Name("td"))
                    .nth(2)?
                    .find(Name("a"))
                    .next()?
                    .attr("href")?
                    .to_owned();
                problems.push((name, href));
            }
            if problems.is_empty() {
                None
            } else {
                Some(problems)
            }
        };
        extract().ok_or(ServiceError::Scrape)
    }

    fn extract_csrf_token_from_submit_page(&self) -> ServiceResult<String> {
        self.find(Attr("id", "submit_form").child(Name("input")))
            .filter(|input| input.attr("name") == Some("csrf_token"))
            .filter_map(|input| input.attr("value").map(ToOwned::to_owned))
            .next()
            .ok_or(ServiceError::Scrape)
    }

    fn extract_url_from_submit_page(&self) -> ServiceResult<String> {
        self.find(Attr("id", "submit_form"))
            .filter_map(|form| form.attr("action").map(ToOwned::to_owned))
            .next()
            .ok_or(ServiceError::Scrape)
    }
}

#[cfg(test)]
mod tests {
    use console::Console;
    use errors::SessionResult;
    use service::session::{HasSession as _HasSession, HttpSession, UrlBase};
    use service::yukicoder::{Extract as _Extract, Username, Yukicoder};
    use service::{self, RevelSession};
    use testsuite::TestSuite;

    use env_logger;
    use url::Host;

    use std::borrow::Borrow;
    use std::io::{BufRead, Write};
    use std::time::Duration;

    #[test]
    #[ignore]
    fn it_extracts_samples_from_problem1() {
        let _ = env_logger::try_init();
        let expected = TestSuite::simple(
            Duration::from_secs(5),
            None,
            None,
            vec![
                ("3\n100\n3\n1 2 1\n2 3 3\n10 90 10\n10 10 50\n", "20\n"),
                ("3\n100\n3\n1 2 1\n2 3 3\n1 100 10\n10 10 50\n", "50\n"),
                (
                    "10\n10\n19\n1 1 2 4 5 1 3 4 6 4 6 4 5 7 8 2 3 4 9\n\
                     3 5 5 5 6 7 7 7 7 8 8 9 9 9 9 10 10 10 10\n\
                     8 6 8 7 6 6 9 9 7 6 9 7 7 8 7 6 6 8 6\n\
                     8 9 10 4 10 3 5 9 3 4 1 8 3 1 3 6 6 10 4\n",
                    "-1\n",
                ),
            ],
        );
        let samples = {
            let mut null = Console::null();
            let mut yukicoder = start(&mut null).unwrap();
            let document = yukicoder.get("/problems/no/1").recv_html().unwrap();
            document.extract_samples().unwrap()
        };
        assert_eq!(expected, samples);
    }

    #[test]
    #[ignore]
    fn it_extracts_problems_names_and_hrefs_from_yukicoder_open_2015_small() {
        static EXPECTED: &[(&str, &str)] = &[
            ("A", "/problems/no/191"),
            ("B", "/problems/no/192"),
            ("C", "/problems/no/193"),
            ("D", "/problems/no/194"),
            ("E", "/problems/no/195"),
            ("F", "/problems/no/196"),
        ];
        let _ = env_logger::try_init();
        let problems = {
            let mut null = Console::null();
            let mut yukicoder = start(&mut null).unwrap();
            let document = yukicoder.get("/contests/100").recv_html().unwrap();
            document.extract_problems().unwrap()
        };
        assert_eq!(own_pairs(EXPECTED), problems);
    }

    fn own_pairs<O: Borrow<B>, B: ToOwned<Owned = O> + ?Sized>(pairs: &[(&B, &B)]) -> Vec<(O, O)> {
        pairs
            .iter()
            .map(|(l, r)| ((*l).to_owned(), (*r).to_owned()))
            .collect()
    }

    fn start<I: BufRead, O: Write, E: Write>(
        console: &mut Console<I, O, E>,
    ) -> SessionResult<Yukicoder<I, O, E>> {
        let client = service::reqwest_client(Duration::from_secs(10))?;
        let base = UrlBase::new(Host::Domain("yukicoder.me"), true, None);
        let session = HttpSession::new(console, client, base, None)?;
        Ok(Yukicoder {
            session,
            username: Username::None,
            credential: RevelSession::None,
        })
    }
}