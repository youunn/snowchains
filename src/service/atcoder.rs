use error::{ServiceError, ServiceErrorKind, ServiceResult, ServiceResultExt};
use service::scraping_session::ScrapingSession;
use testcase::{Cases, TestCaseFileExtension, TestCaseFilePath};

use regex::Regex;
use reqwest::StatusCode;
use select::document::Document;
use select::node::Node;
use select::predicate::{Attr as HtmlAttr, Class, Name, Predicate, Text};
use std::io::{self, Read};
use std::ops::{Deref, DerefMut};
use std::path::Path;
use term::{Attr as TermAttr, color};


pub fn login() -> ServiceResult<()> {
    fn verify(session: &mut ScrapingSession) -> bool {
        static URL: &'static str = "https://practice.contest.atcoder.jp/settings";
        session.http_get(URL).is_ok()
    }

    if let Ok(mut session) = ScrapingSession::from_cookie_file("atcoder") {
        if verify(&mut session) {
            return Ok(println!("Already signed in."));
        }
    }
    AtCoder::login_and_save().map(|_| ())
}


pub fn participate(contest_name: &str) -> ServiceResult<()> {
    AtCoder::load_or_login()?.participate(contest_name)
}


pub fn download(
    contest_name: &str,
    dir_to_save: &Path,
    extension: TestCaseFileExtension,
) -> ServiceResult<()> {
    let mut atcoder = AtCoder::load_or_login()?;
    atcoder.download_all_tasks(contest_name, dir_to_save, extension)
}


struct AtCoder(ScrapingSession);

impl Deref for AtCoder {
    type Target = ScrapingSession;
    fn deref(&self) -> &ScrapingSession {
        &self.0
    }
}

impl DerefMut for AtCoder {
    fn deref_mut(&mut self) -> &mut ScrapingSession {
        &mut self.0
    }
}

impl AtCoder {
    fn load_or_login() -> ServiceResult<Self> {
        fn verify(session: &mut ScrapingSession) -> bool {
            static URL: &'static str = "https://practice.contest.atcoder.jp/settings";
            session.http_get(URL).is_ok()
        }

        if let Ok(mut session) = ScrapingSession::from_cookie_file("atcoder") {
            if verify(&mut session) {
                return Ok(AtCoder(session));
            }
        }
        Self::login_and_save()
    }

    fn participate(&mut self, contest_name: &str) -> ServiceResult<()> {
        let url = format!(
            "https://{}.contest.atcoder.jp/participants/insert",
            contest_name
        );
        self.http_get_expecting(&url, StatusCode::Found).map(|_| ())
    }

    fn download_all_tasks(
        &mut self,
        contest_name: &str,
        dir_to_save: &Path,
        extension: TestCaseFileExtension,
    ) -> ServiceResult<()> {
        let names_and_pathes = {
            let url = format!("http://{}.contest.atcoder.jp/assignments", contest_name);
            extract_names_and_pathes(self.http_get(&url)?).chain_err(
                || "Probably 404",
            )?
        };
        for (alphabet, path) in names_and_pathes {
            let url = format!("http://{}.contest.atcoder.jp{}", contest_name, path);
            match extract_cases(self.http_get(&url)?) {
                Ok(cases) => {
                    cases.save(&TestCaseFilePath::new(
                        &dir_to_save,
                        &alphabet.to_lowercase(),
                        extension,
                    ))?;
                }
                Err(ServiceError(ServiceErrorKind::ScrapingFailed, _)) => {
                    println!("Failed to scrape. Ignoring.");
                }
                Err(e) => return Err(e),
            }
        }
        Ok(self.save()?)
    }

    fn login_and_save() -> ServiceResult<Self> {
        #[derive(Serialize)]
        struct PostData {
            name: String,
            password: String,
        }

        fn post_data() -> ServiceResult<PostData> {
            let (user_id, password) = super::read_username_and_password("User ID")?;
            Ok(PostData {
                name: user_id,
                password: password,
            })
        }

        static URL: &'static str = "https://practice.contest.atcoder.jp/login";
        let mut session = ScrapingSession::new();
        session.http_get(URL)?;
        while let Err(e) = session.http_post_urlencoded(URL, post_data()?, StatusCode::Found) {
            eprint_decorated!(TermAttr::Bold, Some(color::RED), "error: ");
            eprintln!("{:?}", e);
            println!("Failed to sign in. try again.")
        }
        let atcoder = AtCoder(session);
        atcoder.show_username();
        atcoder.save()?;
        Ok(atcoder)
    }

    fn show_username(&self) {
        let username = self.cookie_value("_user_name").unwrap_or_default();
        println!("Hello, {}.", username);
    }

    fn save(&self) -> io::Result<()> {
        self.save_cookie_to_file("atcoder")
    }
}


fn extract_names_and_pathes<R: Read>(html: R) -> ServiceResult<Vec<(String, String)>> {
    fn extract(document: Document) -> Option<Vec<(String, String)>> {
        let mut names_and_pathes = vec![];
        let predicate = HtmlAttr("id", "outer-inner")
            .child(Name("table"))
            .child(Name("tbody"))
            .child(Name("tr"));
        for node in document.find(predicate) {
            let node = try_opt!(node.find(Name("td")).next());
            let node = try_opt!(node.find(Name("a")).next());
            let url = try_opt!(node.attr("href")).to_owned();
            let text = try_opt!(node.find(Text).next()).text();
            names_and_pathes.push((text, url));
        }
        Some(names_and_pathes)
    }

    super::quit_on_failure(extract(Document::from_read(html)?), Vec::is_empty)
}


fn extract_cases<R: Read>(html: R) -> ServiceResult<Cases> {
    fn try_extracting_sample(section_node: Node, regex: &Regex) -> Option<String> {
        let title = try_opt!(section_node.find(Name("h3")).next()).text();
        let sample = try_opt!(section_node.find(Name("pre")).next()).text();
        return_none_unless!(regex.is_match(&title));
        Some(sample)
    }

    fn extract(document: Document) -> Option<Cases> {
        let timelimit = {
            let re_timelimit = Regex::new("\\D*([0-9]+)sec.*").unwrap();
            let predicate = HtmlAttr("id", "outer-inner").child(Name("p")).child(Text);
            let text = try_opt!(document.find(predicate).nth(0)).text();
            let caps = try_opt!(re_timelimit.captures(&text));
            1000 * caps[1].parse::<u64>().unwrap()
        };
        let samples = {
            let re_input = Regex::new("^入力例 ([0-9]+)$").unwrap();
            let re_output = Regex::new("^出力例 ([0-9]+)$").unwrap();
            let predicate = HtmlAttr("id", "task-statement")
                .child(Class("lang"))
                .child(Class("lang-ja"))
                .child(Class("part"))
                .child(Name("section"));
            let (mut samples, mut input_sample) = (vec![], None);
            for node in document.find(predicate) {
                input_sample = if let Some(input_sample) = input_sample {
                    let output_sample = try_opt!(try_extracting_sample(node, &re_output));
                    samples.push((output_sample, input_sample));
                    None
                } else if let Some(input_sample) = try_extracting_sample(node, &re_input) {
                    Some(input_sample)
                } else {
                    None
                };
            }
            samples
        };
        Some(Cases::from_text(timelimit, samples))
    }

    super::quit_on_failure(extract(Document::from_read(html)?), Cases::is_empty)
}
