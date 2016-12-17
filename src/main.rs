// Copyright 2016 Alex Crawford
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#[macro_use]
extern crate clap;
extern crate hyper;
extern crate pulldown_cmark;
extern crate url;
extern crate walkdir;

use clap::{Arg, App};
use hyper::client::Client;
use hyper::header::UserAgent;
use hyper::status::StatusCode;
use pulldown_cmark::{Event, OPTION_ENABLE_TABLES, Parser, Tag};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::exit;
use url::{ParseError, Url};
use walkdir::WalkDir;

macro_rules! fail {
    ($($print:expr),+) => {
        {
            let _ = writeln!(&mut std::io::stderr(), $($print),+);
            exit(2);
        }
    };
}

macro_rules! print_issue {
    ($flag:expr, $($print:expr),+) => {
        {
            println!($($print),+);
            $flag = true;
        }
    };
}

struct State<'a> {
    skip_http: bool,
    code_block: bool,
    last_text: String,
    directory: &'a Path,
}

enum LinkError {
    PathAbsolute,
    PathNonExistant,
    HttpStatus(StatusCode),
    HttpError(hyper::error::Error),
    UrlMalformed(ParseError),
}

fn main() {
    let matches = App::new("Marker")
        .version(crate_version!())
        .arg(Arg::with_name("root")
            .short("r")
            .long("root")
            .help("The path to the root of the documentation to be checked")
            .takes_value(true))
        .arg(Arg::with_name("skip-http")
            .long("skip-http")
            .help("Skip validation of HTTP[S] URLs"))
        .get_matches();

    let skip_http = matches.is_present("skip-http");
    let root = Path::new(matches.value_of("root").unwrap_or("."));
    let mut links = HashMap::new();
    let mut found_issue = false;

    for entry in WalkDir::new(root).into_iter().filter_map(|entry| {
        let entry = entry.unwrap_or_else(|err| {
            fail!("Failed to walk directory: {}", err);
        });
        if entry.file_type().is_file() && entry.path().extension() == Some(OsStr::new("md")) {
            Some(entry)
        } else {
            None
        }
    }) {
        let contents = {
            let mut file = File::open(entry.path()).unwrap_or_else(|err| {
                fail!("Failed to open file ({}): {}", entry.path().display(), err);
            });
            let mut text = String::new();
            if let Err(err) = file.read_to_string(&mut text) {
                fail!("Failed to read file ({}): {}", entry.path().display(), err);
            };
            text
        };

        let newlines = contents.match_indices('\n').map(|(i, _)| i).collect::<Vec<_>>();

        let mut state = State {
            skip_http: skip_http,
            code_block: false,
            last_text: String::new(),
            directory: entry.path().parent().expect("non-root path"),
        };
        let mut parser = Parser::new_ext(&contents, OPTION_ENABLE_TABLES);
        while let Some(event) = parser.next() {
            let line = newlines.iter().take_while(|i| i < &&parser.get_offset()).count() + 1;
            match event {
                Event::Text(ref text) => {
                    state.last_text = text.to_string();
                    if let Some(reference) = try_reference(&state, text) {
                        print_issue!(found_issue,
                                     "Found broken reference ({}) in {}:{}",
                                     reference,
                                     entry.path().display(),
                                     line)
                    }
                }
                Event::End(Tag::Link(dest, _)) => {
                    match check_link(&mut state, &mut links, &dest) {
                        Ok(()) => {}
                        Err(LinkError::PathAbsolute) => {
                            print_issue!(found_issue,
                                         "Found absolute path    ({} -> {}) at {}:{}",
                                         state.last_text,
                                         dest,
                                         entry.path().display(),
                                         line)
                        }
                        Err(LinkError::PathNonExistant) => {
                            print_issue!(found_issue,
                                         "Found broken path      ({} -> {}) at {}:{}",
                                         state.last_text,
                                         dest,
                                         entry.path().display(),
                                         line)
                        }
                        Err(LinkError::HttpStatus(status)) => {
                            print_issue!(found_issue,
                                         "Found broken url       ({} -> {}) at {}:{} : {}",
                                         state.last_text,
                                         dest,
                                         entry.path().display(),
                                         line,
                                         status)
                        }
                        Err(LinkError::HttpError(err)) => {
                            print_issue!(found_issue,
                                         "HTTP failure           ({} -> {}) at {}:{} : {}",
                                         state.last_text,
                                         dest,
                                         entry.path().display(),
                                         line,
                                         err);
                        }
                        Err(LinkError::UrlMalformed(err)) => {
                            print_issue!(found_issue,
                                         "Found malformed URL    ({} -> {}) at {}:{} : {}",
                                         state.last_text,
                                         dest,
                                         entry.path().display(),
                                         line,
                                         err);
                        }
                    }
                }
                Event::Start(Tag::Code) |
                Event::Start(Tag::CodeBlock(_)) => state.code_block = true,
                Event::End(Tag::Code) |
                Event::End(Tag::CodeBlock(_)) => state.code_block = false,
                _ => {}
            }
        }
    }

    if found_issue {
        exit(1)
    }
}

fn try_reference<'a>(state: &State, text: &'a str) -> Option<&'a str> {
    if !state.code_block && text.chars().next() == Some('[') && text.chars().last() == Some(']') {
        Some(&text[1..(text.len() - 1)])
    } else {
        None
    }
}

fn check_link(state: &mut State,
              links: &mut HashMap<Url, StatusCode>,
              dest: &str)
              -> Result<(), LinkError> {
    match (Url::parse(&dest), state.skip_http) {
        (Ok(url), false) => check_url(links, url),
        (Ok(_), true) => Ok(()),
        (Err(ParseError::RelativeUrlWithoutBase), _) => check_path(state, dest),
        (Err(err), _) => Err(LinkError::UrlMalformed(err)),
    }
}

fn check_url(links: &mut HashMap<Url, StatusCode>, url: Url) -> Result<(), LinkError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Ok(());
    }

    let client = Client::new();
    let agent = UserAgent(format!("marker/{}", crate_version!()));

    let status = if let Some(status) = links.remove(&url) {
        status
    } else {
        let res = client.head(url.clone()).header(agent.clone()).send().and_then(|resp| {
            if resp.status == StatusCode::MethodNotAllowed {
                client.get(url.clone()).header(agent.clone()).send()
            } else {
                Ok(resp)
            }
        });
        match res {
            Ok(resp) => resp.status,
            Err(err) => return Err(LinkError::HttpError(err)),
        }
    };
    links.insert(url, status);
    match status {
        StatusCode::Ok => Ok(()),
        status => Err(LinkError::HttpStatus(status)),
    }
}

fn check_path(state: &mut State, path: &str) -> Result<(), LinkError> {
    let path = Path::new(OsStr::new(path.split('#').next().expect("string")));
    if path.is_absolute() {
        Err(LinkError::PathAbsolute)
    } else if !state.directory.join(path).exists() {
        Err(LinkError::PathNonExistant)
    } else {
        Ok(())
    }
}
